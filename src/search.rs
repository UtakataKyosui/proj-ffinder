use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::path::Path;

use regex::RegexBuilder;

use crate::model::{
    FileSummary, GrepContextLine, GrepMatch, GrepMode, GrepQuery, MatchedTag, SearchAssistance,
    SearchHit, SearchQuery, SearchResult, SuggestedCommand, SuggestedTag, Tag,
};
use crate::scanner;

const MAX_SUGGESTED_TAGS: usize = 5;
const MAX_SAMPLED_PATHS_PER_TAG: usize = 2;

struct PreparedGrep {
    query: GrepQuery,
    matcher: regex::Regex,
}

pub fn search_files(
    root: &str,
    files: &[FileSummary],
    query: SearchQuery,
) -> Result<SearchResult, regex::Error> {
    let root_path = Path::new(root);
    let prepared_grep = prepare_grep(query.grep.as_ref())?;
    let mut hits = Vec::new();
    let mut assistance_inputs = Vec::new();
    for file in files {
        let Some(hit) = score_file(root_path, file, &query, prepared_grep.as_ref()) else {
            continue;
        };
        assistance_inputs.push(AssistanceInput {
            path: file.path.clone(),
            tags: file.tags.clone(),
        });
        hits.push(hit);
    }
    let assistance = build_search_assistance(root_path, &query, &assistance_inputs);

    Ok(SearchResult {
        root: root.to_owned(),
        query,
        hits,
        assistance,
    })
}

pub fn search_files_lazy(
    root: &Path,
    query: SearchQuery,
) -> Result<Option<SearchResult>, Box<dyn Error>> {
    if !supports_lazy_search(&query) {
        return Ok(None);
    }

    let Some(paths) = scanner::load_index_paths(root)? else {
        return Ok(None);
    };
    let prepared_grep = prepare_grep(query.grep.as_ref())?;
    let Some(prepared_grep) = prepared_grep.as_ref() else {
        return Ok(None);
    };

    let mut hits = Vec::new();
    let mut assistance_inputs = Vec::new();
    for relative_path in paths {
        let Some(grep_matches) = collect_grep_matches_for_path(root, &relative_path, prepared_grep)
        else {
            continue;
        };
        let Some(file) = scanner::load_cached_summary(root, &relative_path)? else {
            return Ok(None);
        };
        assistance_inputs.push(AssistanceInput {
            path: file.path.clone(),
            tags: file.tags.clone(),
        });
        hits.push(SearchHit {
            file_id: file.file_id,
            path: file.path,
            kind: file.kind,
            language: file.language,
            score: grep_matches.len() as f32 * 25.0,
            matched_must: Vec::new(),
            matched_must_details: Vec::new(),
            matched_any: Vec::new(),
            matched_any_details: Vec::new(),
            matched_prefer: Vec::new(),
            matched_prefer_details: Vec::new(),
            grep_matches,
        });
    }
    let assistance = build_search_assistance(root, &query, &assistance_inputs);

    Ok(Some(SearchResult {
        root: root.display().to_string(),
        query,
        hits,
        assistance,
    }))
}

#[derive(Debug)]
struct AssistanceInput {
    path: String,
    tags: Vec<Tag>,
}

fn score_file(
    root: &Path,
    file: &FileSummary,
    query: &SearchQuery,
    prepared_grep: Option<&PreparedGrep>,
) -> Option<SearchHit> {
    let tags = file
        .tags
        .iter()
        .map(|tag| (tag.value.as_str(), tag))
        .collect::<HashMap<_, _>>();

    if query
        .exclude
        .iter()
        .any(|tag| tags.contains_key(tag.as_str()))
    {
        return None;
    }

    let matched_must = query
        .must
        .iter()
        .filter(|tag| tags.contains_key(tag.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if matched_must.len() != query.must.len() {
        return None;
    }

    let matched_any = query
        .any
        .iter()
        .filter(|tag| tags.contains_key(tag.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !query.any.is_empty() && matched_any.is_empty() {
        return None;
    }

    let matched_prefer = query
        .prefer
        .iter()
        .filter(|tag| tags.contains_key(tag.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let matched_must_details = matched_details(&matched_must, &tags);
    let matched_any_details = matched_details(&matched_any, &tags);
    let matched_prefer_details = matched_details(&matched_prefer, &tags);
    let grep_matches = if let Some(grep) = prepared_grep {
        collect_grep_matches(root, file, grep)?
    } else {
        Vec::new()
    };

    let mut score = 0.0;
    score += matched_must
        .iter()
        .filter_map(|tag| tags.get(tag.as_str()).map(|tag| tag.confidence))
        .sum::<f32>()
        * 10.0;
    score += matched_any
        .iter()
        .filter_map(|tag| tags.get(tag.as_str()).map(|tag| tag.confidence))
        .sum::<f32>()
        * 20.0;
    score += matched_prefer
        .iter()
        .filter_map(|tag| tags.get(tag.as_str()).map(|tag| tag.confidence))
        .sum::<f32>()
        * 5.0;
    score += grep_matches.len() as f32 * 25.0;

    Some(SearchHit {
        file_id: file.file_id.clone(),
        path: file.path.clone(),
        kind: file.kind.clone(),
        language: file.language.clone(),
        score,
        matched_must,
        matched_must_details,
        matched_any,
        matched_any_details,
        matched_prefer,
        matched_prefer_details,
        grep_matches,
    })
}

fn matched_details(matched: &[String], tags: &HashMap<&str, &Tag>) -> Vec<MatchedTag> {
    matched
        .iter()
        .filter_map(|value| tags.get(value.as_str()))
        .map(|tag| MatchedTag {
            value: tag.value.clone(),
            source: tag.source.clone(),
            confidence: tag.confidence,
            evidence: tag.evidence.clone(),
        })
        .collect()
}

fn collect_grep_matches(
    root: &Path,
    file: &FileSummary,
    grep: &PreparedGrep,
) -> Option<Vec<GrepMatch>> {
    collect_grep_matches_for_path(root, &file.path, grep)
}

fn collect_grep_matches_for_path(
    root: &Path,
    relative_path: &str,
    grep: &PreparedGrep,
) -> Option<Vec<GrepMatch>> {
    let path = scanner::join_safe_relative_path(root, relative_path)?;
    let content = fs::read_to_string(path).ok()?;
    let lines = content.lines().collect::<Vec<_>>();

    let mut matches = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if !grep.matcher.is_match(line) {
            continue;
        }

        matches.push(GrepMatch {
            line_number: index + 1,
            line: (*line).to_owned(),
            before: context_lines(&lines, index, grep.query.context_before, true),
            after: context_lines(&lines, index, grep.query.context_after, false),
        });

        if matches.len() >= grep.query.max_matches_per_file {
            break;
        }
    }

    if matches.is_empty() {
        None
    } else {
        Some(matches)
    }
}

fn supports_lazy_search(query: &SearchQuery) -> bool {
    query.grep.is_some()
        && query.must.is_empty()
        && query.any.is_empty()
        && query.exclude.is_empty()
        && query.prefer.is_empty()
}

fn build_search_assistance(
    root: &Path,
    query: &SearchQuery,
    inputs: &[AssistanceInput],
) -> SearchAssistance {
    let excluded_tags = query
        .must
        .iter()
        .chain(query.any.iter())
        .chain(query.exclude.iter())
        .chain(query.prefer.iter())
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut tag_counts = HashMap::<String, usize>::new();
    let mut sample_paths = HashMap::<String, Vec<String>>::new();

    for input in inputs {
        let mut seen_in_hit = HashSet::new();
        for tag in &input.tags {
            if excluded_tags.contains(tag.value.as_str()) || !is_suggestible_tag(&tag.value) {
                continue;
            }
            if !seen_in_hit.insert(tag.value.as_str()) {
                continue;
            }
            *tag_counts.entry(tag.value.clone()).or_default() += 1;
            let paths = sample_paths.entry(tag.value.clone()).or_default();
            if paths.len() < MAX_SAMPLED_PATHS_PER_TAG
                && !paths.iter().any(|path| path == &input.path)
            {
                paths.push(input.path.clone());
            }
        }
    }

    let mut suggested_tags = tag_counts
        .into_iter()
        .map(|(value, hit_count)| SuggestedTag {
            sample_paths: sample_paths.remove(&value).unwrap_or_default(),
            value,
            hit_count,
        })
        .collect::<Vec<_>>();
    suggested_tags.sort_by(|left, right| {
        right
            .hit_count
            .cmp(&left.hit_count)
            .then_with(|| tag_priority(&left.value).cmp(&tag_priority(&right.value)))
            .then_with(|| left.value.cmp(&right.value))
    });
    suggested_tags.truncate(MAX_SUGGESTED_TAGS);

    let suggested_commands = build_suggested_commands(root, query, &suggested_tags);

    SearchAssistance {
        suggested_tags,
        suggested_commands,
    }
}

fn build_suggested_commands(
    root: &Path,
    query: &SearchQuery,
    suggested_tags: &[SuggestedTag],
) -> Vec<SuggestedCommand> {
    let mut commands = Vec::new();
    let root_hint = shell_quote(if root == Path::new(".") {
        ".".to_owned()
    } else {
        root.display().to_string()
    });

    if let Some(first) = suggested_tags.first() {
        let first_tag = shell_quote(&first.value);
        commands.push(SuggestedCommand {
            description: format!("narrow with {}", first.value),
            command: format!(
                "proj-finder search {root_hint} --must {} --limit {}",
                first_tag, query.limit
            ),
        });
    }

    if suggested_tags.len() >= 2 {
        let first_tag = shell_quote(&suggested_tags[0].value);
        let second_tag = shell_quote(&suggested_tags[1].value);
        commands.push(SuggestedCommand {
            description: format!(
                "narrow with {} and {}",
                suggested_tags[0].value, suggested_tags[1].value
            ),
            command: format!(
                "proj-finder search {root_hint} --must {} --must {} --limit {}",
                first_tag, second_tag, query.limit
            ),
        });
    }

    commands
}

fn shell_quote(value: impl AsRef<str>) -> String {
    let value = value.as_ref();
    if value.is_empty() {
        return "''".to_owned();
    }

    if value
        .bytes()
        .all(|byte| matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'/' | b'.' | b'_' | b':' | b'-'))
    {
        return value.to_owned();
    }

    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn is_suggestible_tag(value: &str) -> bool {
    matches!(
        tag_prefix(value),
        Some("role")
            | Some("tech")
            | Some("kw")
            | Some("import")
            | Some("export")
            | Some("symbol")
            | Some("side-effect")
            | Some("kind")
            | Some("lang")
    )
}

fn tag_priority(value: &str) -> usize {
    match tag_prefix(value) {
        Some("role") => 0,
        Some("tech") => 1,
        Some("kw") => 2,
        Some("import") => 3,
        Some("export") => 4,
        Some("symbol") => 5,
        Some("side-effect") => 6,
        Some("kind") => 7,
        Some("lang") => 8,
        _ => 9,
    }
}

fn tag_prefix(value: &str) -> Option<&str> {
    value.split_once(':').map(|(prefix, _)| prefix)
}

fn prepare_grep(grep: Option<&GrepQuery>) -> Result<Option<PreparedGrep>, regex::Error> {
    grep.map(|query| {
        Ok(PreparedGrep {
            query: query.clone(),
            matcher: build_matcher(query)?,
        })
    })
    .transpose()
}

fn build_matcher(grep: &GrepQuery) -> Result<regex::Regex, regex::Error> {
    let pattern = match grep.mode {
        GrepMode::Literal => regex::escape(&grep.pattern),
        GrepMode::Regex => grep.pattern.clone(),
    };

    RegexBuilder::new(&pattern)
        .case_insensitive(!grep.case_sensitive)
        .build()
}

fn context_lines(
    lines: &[&str],
    matched_index: usize,
    context_size: usize,
    before: bool,
) -> Vec<GrepContextLine> {
    if context_size == 0 {
        return Vec::new();
    }

    let range = if before {
        matched_index.saturating_sub(context_size)..matched_index
    } else {
        (matched_index + 1)..usize::min(lines.len(), matched_index + 1 + context_size)
    };

    range
        .map(|index| GrepContextLine {
            line_number: index + 1,
            line: lines[index].to_owned(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::model::{FileSummary, GrepMode, GrepQuery, LocationSummary, SearchQuery, Tag};
    use crate::scanner::{self, ScanMode};

    use super::{
        PreparedGrep, build_search_assistance, collect_grep_matches_for_path, search_files,
        search_files_lazy, shell_quote,
    };

    fn temp_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("proj-finder-search-{name}-{nanos}"))
    }

    fn sample_file(path: &str, kind: &str, tags: &[&str]) -> FileSummary {
        FileSummary {
            file_id: path.to_owned(),
            path: path.to_owned(),
            language: Some("rust".to_owned()),
            kind: kind.to_owned(),
            location_summary: LocationSummary {
                depth: 2,
                dirs: vec!["src".to_owned()],
                basename: "main.rs".to_owned(),
                stem: "main".to_owned(),
                extension: Some("rs".to_owned()),
                path_tokens: vec!["src".to_owned(), "main".to_owned(), "rs".to_owned()],
                path_patterns: vec!["src/*".to_owned()],
            },
            content_summary: None,
            tags: tags
                .iter()
                .map(|value| Tag {
                    value: (*value).to_owned(),
                    source: "test".to_owned(),
                    confidence: 1.0,
                    evidence: vec![],
                })
                .collect(),
        }
    }

    #[test]
    fn search_filters_by_must_and_any() {
        let files = vec![
            sample_file(
                "src/auth/route.rs",
                "source",
                &["role:api-route", "kw:auth"],
            ),
            sample_file(
                "src/user/route.rs",
                "source",
                &["role:api-route", "kw:user"],
            ),
        ];
        let query = SearchQuery {
            must: vec!["role:api-route".to_owned()],
            any: vec!["kw:auth".to_owned()],
            exclude: vec![],
            prefer: vec![],
            grep: None,
            limit: 10,
        };

        let result = search_files("/tmp/project", &files, query).expect("search should succeed");

        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.hits[0].path, "src/auth/route.rs");
        assert_eq!(result.hits[0].matched_must_details.len(), 1);
        assert_eq!(result.hits[0].matched_any_details.len(), 1);
    }

    #[test]
    fn search_excludes_matching_tags() {
        let files = vec![
            sample_file(
                "src/auth/route.rs",
                "source",
                &["role:api-route", "kw:auth"],
            ),
            sample_file(
                "src/generated/auth/route.rs",
                "source",
                &["role:api-route", "kw:auth", "kind:generated"],
            ),
        ];
        let query = SearchQuery {
            must: vec!["role:api-route".to_owned()],
            any: vec!["kw:auth".to_owned()],
            exclude: vec!["kind:generated".to_owned()],
            prefer: vec![],
            grep: None,
            limit: 10,
        };

        let result = search_files("/tmp/project", &files, query).expect("search should succeed");

        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.hits[0].path, "src/auth/route.rs");
    }

    #[test]
    fn search_ranks_preferred_files_higher() {
        let files = vec![
            sample_file(
                "src/auth/route.rs",
                "source",
                &["role:api-route", "kw:auth", "dir:src", "tech:http"],
            ),
            sample_file(
                "src/auth/handler.rs",
                "source",
                &["role:api-route", "kw:auth"],
            ),
        ];
        let query = SearchQuery {
            must: vec!["role:api-route".to_owned()],
            any: vec!["kw:auth".to_owned()],
            exclude: vec![],
            prefer: vec!["tech:http".to_owned()],
            grep: None,
            limit: 10,
        };

        let result = search_files("/tmp/project", &files, query).expect("search should succeed");

        assert_eq!(result.hits.len(), 2);
        assert_eq!(result.hits[0].path, "src/auth/route.rs");
        assert!(result.hits[0].score > result.hits[1].score);
        assert_eq!(result.hits[0].matched_prefer_details.len(), 1);
    }

    #[test]
    fn search_filters_by_literal_grep_and_returns_context() {
        let root = temp_path("literal-grep");
        fs::create_dir_all(root.join("src")).expect("should create src dir");
        fs::write(
            root.join("src").join("main.rs"),
            "fn helper() {}\nfn main() {\n    println!(\"hello\");\n}\n",
        )
        .expect("should write source file");

        let files = vec![sample_file("src/main.rs", "source", &["lang:rust"])];
        let query = SearchQuery {
            must: vec![],
            any: vec![],
            exclude: vec![],
            prefer: vec![],
            grep: Some(GrepQuery {
                pattern: "println!".to_owned(),
                mode: GrepMode::Literal,
                case_sensitive: true,
                context_before: 1,
                context_after: 1,
                max_matches_per_file: 3,
            }),
            limit: 10,
        };

        let result = search_files(
            root.to_str().expect("temp path should be utf-8"),
            &files,
            query,
        )
        .expect("search should succeed");

        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.hits[0].grep_matches.len(), 1);
        assert_eq!(result.hits[0].grep_matches[0].line_number, 3);
        assert_eq!(result.hits[0].grep_matches[0].before[0].line_number, 2);
        assert_eq!(result.hits[0].grep_matches[0].after[0].line_number, 4);

        fs::remove_dir_all(root).expect("should clean up temp dir");
    }

    #[test]
    fn search_filters_by_regex_grep_case_insensitive() {
        let root = temp_path("regex-grep");
        fs::create_dir_all(root.join("src")).expect("should create src dir");
        fs::write(
            root.join("src").join("main.rs"),
            "export function loadUser() {}\nconst value = 1;\n",
        )
        .expect("should write source file");

        let files = vec![sample_file("src/main.rs", "source", &["lang:typescript"])];
        let query = SearchQuery {
            must: vec![],
            any: vec![],
            exclude: vec![],
            prefer: vec![],
            grep: Some(GrepQuery {
                pattern: "load[a-z]+".to_owned(),
                mode: GrepMode::Regex,
                case_sensitive: false,
                context_before: 0,
                context_after: 0,
                max_matches_per_file: 3,
            }),
            limit: 10,
        };

        let result = search_files(
            root.to_str().expect("temp path should be utf-8"),
            &files,
            query,
        )
        .expect("search should succeed");

        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.hits[0].grep_matches[0].line_number, 1);

        fs::remove_dir_all(root).expect("should clean up temp dir");
    }

    #[test]
    fn search_returns_error_for_invalid_regex() {
        let files = vec![sample_file("src/main.rs", "source", &["lang:rust"])];
        let query = SearchQuery {
            must: vec![],
            any: vec![],
            exclude: vec![],
            prefer: vec![],
            grep: Some(GrepQuery {
                pattern: "(unterminated".to_owned(),
                mode: GrepMode::Regex,
                case_sensitive: true,
                context_before: 0,
                context_after: 0,
                max_matches_per_file: 3,
            }),
            limit: 10,
        };

        assert!(search_files("/tmp/project", &files, query).is_err());
    }

    #[test]
    fn lazy_search_reads_manifest_for_grep_only_queries() {
        let root = temp_path("lazy-grep");
        fs::create_dir_all(root.join("src")).expect("should create src dir");
        fs::write(
            root.join("src").join("main.rs"),
            "fn main() {\n    println!(\"hello\");\n}\n",
        )
        .expect("should write main source file");
        fs::write(root.join("src").join("lib.rs"), "pub fn helper() {}\n")
            .expect("should write lib source file");

        scanner::scan_project_with_mode(&root, ScanMode::Full).expect("full scan should persist");

        let query = SearchQuery {
            must: vec![],
            any: vec![],
            exclude: vec![],
            prefer: vec![],
            grep: Some(GrepQuery {
                pattern: "println!".to_owned(),
                mode: GrepMode::Literal,
                case_sensitive: true,
                context_before: 0,
                context_after: 0,
                max_matches_per_file: 3,
            }),
            limit: 10,
        };

        let result = search_files_lazy(&root, query)
            .expect("lazy search should succeed")
            .expect("grep-only query should use lazy path");

        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.hits[0].path, "src/main.rs");
        assert_eq!(result.hits[0].grep_matches[0].line_number, 2);

        fs::remove_dir_all(root).expect("should clean up temp dir");
    }

    #[test]
    fn lazy_search_returns_none_for_tag_filtered_queries() {
        let root = temp_path("lazy-tag-filter");
        fs::create_dir_all(root.join("src")).expect("should create src dir");
        fs::write(
            root.join("src").join("main.rs"),
            "fn main() {\n    println!(\"hello\");\n}\n",
        )
        .expect("should write main source file");

        scanner::scan_project_with_mode(&root, ScanMode::Full).expect("full scan should persist");

        let query = SearchQuery {
            must: vec!["lang:rust".to_owned()],
            any: vec![],
            exclude: vec![],
            prefer: vec![],
            grep: Some(GrepQuery {
                pattern: "println!".to_owned(),
                mode: GrepMode::Literal,
                case_sensitive: true,
                context_before: 0,
                context_after: 0,
                max_matches_per_file: 3,
            }),
            limit: 10,
        };

        let result = search_files_lazy(&root, query).expect("lazy search should not error");
        assert!(result.is_none());

        fs::remove_dir_all(root).expect("should clean up temp dir");
    }

    #[test]
    fn grep_match_collection_rejects_unsafe_relative_paths() {
        let root = temp_path("unsafe-grep-path");
        fs::create_dir_all(&root).expect("should create temp dir");
        let grep = PreparedGrep {
            query: GrepQuery {
                pattern: "hello".to_owned(),
                mode: GrepMode::Literal,
                case_sensitive: true,
                context_before: 0,
                context_after: 0,
                max_matches_per_file: 1,
            },
            matcher: regex::Regex::new("hello").expect("regex should compile"),
        };

        assert!(collect_grep_matches_for_path(&root, "../outside", &grep).is_none());

        fs::remove_dir_all(root).expect("should clean up temp dir");
    }

    #[test]
    fn shell_quote_quotes_paths_with_spaces() {
        assert_eq!(shell_quote("."), ".");
        assert_eq!(shell_quote("/tmp/my repo"), "'/tmp/my repo'");
        assert_eq!(shell_quote("tag:'quoted'"), "'tag:'\"'\"'quoted'\"'\"''");
    }

    #[test]
    fn search_assistance_quotes_suggested_commands() {
        let root = Path::new("/tmp/my repo");
        let query = SearchQuery {
            must: vec![],
            any: vec![],
            exclude: vec![],
            prefer: vec![],
            grep: None,
            limit: 5,
        };
        let inputs = vec![
            super::AssistanceInput {
                path: "src/main.rs".to_owned(),
                tags: vec![Tag {
                    value: "tech:react app".to_owned(),
                    source: "test".to_owned(),
                    confidence: 1.0,
                    evidence: vec![],
                }],
            },
            super::AssistanceInput {
                path: "src/app.tsx".to_owned(),
                tags: vec![Tag {
                    value: "tech:react app".to_owned(),
                    source: "test".to_owned(),
                    confidence: 1.0,
                    evidence: vec![],
                }],
            },
        ];

        let assistance = build_search_assistance(root, &query, &inputs);

        assert_eq!(assistance.suggested_tags.len(), 1);
        assert_eq!(
            assistance.suggested_commands[0].command,
            "proj-finder search '/tmp/my repo' --must 'tech:react app' --limit 5"
        );
    }
}
