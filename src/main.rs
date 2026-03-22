mod formatter;
mod model;
mod polyglot_ast;
mod rust_ast;
mod scanner;
mod search;

use std::collections::HashSet;
use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use model::{GrepMode, GrepQuery, SearchQuery};
use scanner::ScanMode;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum SearchDisplay {
    Path,
    #[default]
    Summary,
    Full,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum SearchSort {
    Path,
    #[default]
    Score,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum SearchReasonDisplay {
    None,
    Brief,
    #[default]
    Full,
}

#[derive(Debug, Parser)]
#[command(name = "proj-finder")]
#[command(about = "Project file summarizer and tag-based search CLI")]
#[command(args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    // Keep the old `proj-finder .` behavior by treating a lone positional
    // argument as the repo root hint when no subcommand is provided.
    #[arg(value_name = "ROOT")]
    root: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
#[allow(clippy::large_enum_variant)]
enum Command {
    Scan {
        #[arg(value_name = "ROOT")]
        root: Option<PathBuf>,
        #[arg(long)]
        incremental: bool,
    },
    Search {
        #[arg(value_name = "ROOT")]
        root: Option<PathBuf>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        no_color: bool,
        #[arg(long, value_enum, default_value_t = SearchDisplay::Summary)]
        show: SearchDisplay,
        #[arg(long, value_enum, default_value_t = SearchSort::Score)]
        sort: SearchSort,
        #[arg(long, value_enum, default_value_t = SearchReasonDisplay::Full)]
        show_reason: SearchReasonDisplay,
        #[arg(long, value_name = "JSON")]
        query: Option<String>,
        #[arg(long, value_name = "LANG", action = ArgAction::Append)]
        lang: Vec<String>,
        #[arg(long, value_name = "LANG", action = ArgAction::Append)]
        exclude_lang: Vec<String>,
        #[arg(long, value_name = "KIND", action = ArgAction::Append)]
        kind: Vec<String>,
        #[arg(long, value_name = "KIND", action = ArgAction::Append)]
        exclude_kind: Vec<String>,
        #[arg(long, value_name = "ROLE", action = ArgAction::Append)]
        role: Vec<String>,
        #[arg(long, value_name = "ROLE", action = ArgAction::Append)]
        exclude_role: Vec<String>,
        #[arg(long, value_name = "TECH", action = ArgAction::Append)]
        tech: Vec<String>,
        #[arg(long, value_name = "TECH", action = ArgAction::Append)]
        exclude_tech: Vec<String>,
        #[arg(long, value_name = "KEYWORD", action = ArgAction::Append)]
        keyword: Vec<String>,
        #[arg(long, value_name = "KEYWORD", action = ArgAction::Append)]
        exclude_keyword: Vec<String>,
        #[arg(long, value_name = "TAG", action = ArgAction::Append)]
        must: Vec<String>,
        #[arg(long, value_name = "TAG", action = ArgAction::Append)]
        any: Vec<String>,
        #[arg(long, value_name = "TAG", action = ArgAction::Append)]
        exclude: Vec<String>,
        #[arg(long, value_name = "TAG", action = ArgAction::Append)]
        prefer: Vec<String>,
        #[arg(long, value_name = "PATTERN")]
        grep: Option<String>,
        #[arg(long)]
        regex: bool,
        #[arg(long)]
        case_sensitive: bool,
        #[arg(long, default_value_t = 0)]
        context_before: usize,
        #[arg(long, default_value_t = 0)]
        context_after: usize,
        #[arg(long, default_value_t = 3)]
        max_matches_per_file: usize,
        #[arg(long, default_value_t = 10)]
        limit: usize,
        #[arg(long)]
        incremental: bool,
    },
    Grep {
        #[arg(value_name = "PATTERN")]
        pattern: String,
        #[arg(value_name = "ROOT")]
        root: Option<PathBuf>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        no_color: bool,
        #[arg(long)]
        regex: bool,
        #[arg(long)]
        case_sensitive: bool,
        #[arg(long, default_value_t = 0)]
        context_before: usize,
        #[arg(long, default_value_t = 0)]
        context_after: usize,
        #[arg(long, default_value_t = 3)]
        max_matches_per_file: usize,
        #[arg(long, default_value_t = 10)]
        limit: usize,
        #[arg(long)]
        incremental: bool,
    },
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Scan { root, incremental }) => run_scan(root, incremental)?,
        Some(Command::Search {
            root,
            json,
            no_color,
            show,
            sort,
            show_reason,
            query,
            lang,
            exclude_lang,
            kind,
            exclude_kind,
            role,
            exclude_role,
            tech,
            exclude_tech,
            keyword,
            exclude_keyword,
            must,
            any,
            exclude,
            prefer,
            grep,
            regex,
            case_sensitive,
            context_before,
            context_after,
            max_matches_per_file,
            limit,
            incremental,
        }) => run_search(
            root,
            json,
            no_color,
            show,
            sort,
            show_reason,
            query,
            lang,
            exclude_lang,
            kind,
            exclude_kind,
            role,
            exclude_role,
            tech,
            exclude_tech,
            keyword,
            exclude_keyword,
            must,
            any,
            exclude,
            prefer,
            grep,
            regex,
            case_sensitive,
            context_before,
            context_after,
            max_matches_per_file,
            limit,
            incremental,
        )?,
        Some(Command::Grep {
            pattern,
            root,
            json,
            no_color,
            regex,
            case_sensitive,
            context_before,
            context_after,
            max_matches_per_file,
            limit,
            incremental,
        }) => run_grep(
            root,
            &pattern,
            json,
            no_color,
            regex,
            case_sensitive,
            context_before,
            context_after,
            max_matches_per_file,
            limit,
            incremental,
        )?,
        None => run_scan(cli.root, false)?,
    }

    println!();
    Ok(())
}

fn run_scan(root: Option<PathBuf>, incremental: bool) -> Result<(), Box<dyn Error>> {
    let root = resolve_repo_root(root)?;
    let scan_result = scan_result_for(&root, incremental)?;
    serde_json::to_writer_pretty(std::io::stdout(), &scan_result)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_search(
    root: Option<PathBuf>,
    json: bool,
    no_color: bool,
    show: SearchDisplay,
    sort: SearchSort,
    show_reason: SearchReasonDisplay,
    query_json: Option<String>,
    lang: Vec<String>,
    exclude_lang: Vec<String>,
    kind: Vec<String>,
    exclude_kind: Vec<String>,
    role: Vec<String>,
    exclude_role: Vec<String>,
    tech: Vec<String>,
    exclude_tech: Vec<String>,
    keyword: Vec<String>,
    exclude_keyword: Vec<String>,
    must: Vec<String>,
    any: Vec<String>,
    exclude: Vec<String>,
    prefer: Vec<String>,
    grep: Option<String>,
    regex: bool,
    case_sensitive: bool,
    context_before: usize,
    context_after: usize,
    max_matches_per_file: usize,
    limit: usize,
    incremental: bool,
) -> Result<(), Box<dyn Error>> {
    let root = resolve_repo_root(root)?;
    let query = build_search_query(
        query_json,
        lang,
        exclude_lang,
        kind,
        exclude_kind,
        role,
        exclude_role,
        tech,
        exclude_tech,
        keyword,
        exclude_keyword,
        must,
        any,
        exclude,
        prefer,
        grep,
        regex,
        case_sensitive,
        context_before,
        context_after,
        max_matches_per_file,
        limit,
    )?;
    let mut search_result = if incremental {
        let root = scanner::prepare_project_with_mode(&root, ScanMode::Incremental)?;
        if let Some(search_result) = search::search_files_lazy(&root, query.clone())? {
            search_result
        } else {
            let scan_result = scanner::load_scan_result(&root)?.unwrap_or(
                scanner::scan_project_with_mode(&root, ScanMode::Incremental)?,
            );
            search::search_files(&scan_result.root, &scan_result.files, query)?
        }
    } else {
        let scan_result = scan_result_for(&root, false)?;
        search::search_files(&scan_result.root, &scan_result.files, query)?
    };
    finalize_search_result(&mut search_result, sort);

    if json {
        serde_json::to_writer_pretty(std::io::stdout(), &search_result)?;
    } else {
        print!(
            "{}",
            formatter::format_search_result(&search_result, !no_color, show, show_reason)?
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_grep(
    root: Option<PathBuf>,
    pattern: &str,
    json: bool,
    no_color: bool,
    regex: bool,
    case_sensitive: bool,
    context_before: usize,
    context_after: usize,
    max_matches_per_file: usize,
    limit: usize,
    incremental: bool,
) -> Result<(), Box<dyn Error>> {
    let root = resolve_repo_root(root)?;
    let query = grep_search_query(
        pattern.to_owned(),
        regex,
        case_sensitive,
        context_before,
        context_after,
        max_matches_per_file,
        limit,
    );
    let mut search_result = if incremental {
        let root = scanner::prepare_project_with_mode(&root, ScanMode::Incremental)?;
        if let Some(search_result) = search::search_files_lazy(&root, query.clone())? {
            search_result
        } else {
            let scan_result = scanner::load_scan_result(&root)?.unwrap_or(
                scanner::scan_project_with_mode(&root, ScanMode::Incremental)?,
            );
            search::search_files(&scan_result.root, &scan_result.files, query)?
        }
    } else {
        let scan_result = scan_result_for(&root, false)?;
        search::search_files(&scan_result.root, &scan_result.files, query)?
    };
    finalize_search_result(&mut search_result, SearchSort::Score);

    if json {
        serde_json::to_writer_pretty(std::io::stdout(), &search_result)?;
    } else {
        print!(
            "{}",
            formatter::format_grep_result(&search_result, !no_color)?
        );
    }
    Ok(())
}

fn finalize_search_result(result: &mut model::SearchResult, sort: SearchSort) {
    match sort {
        SearchSort::Score => {
            result.hits.sort_by(|left, right| {
                right
                    .score
                    .partial_cmp(&left.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| left.path.cmp(&right.path))
            });
        }
        SearchSort::Path => {
            result
                .hits
                .sort_by(|left, right| left.path.cmp(&right.path));
        }
    }
    result.hits.truncate(result.query.limit);
}

fn scan_result_for(root: &Path, incremental: bool) -> Result<model::ScanResult, Box<dyn Error>> {
    let mode = if incremental {
        ScanMode::Incremental
    } else {
        ScanMode::Full
    };
    Ok(scanner::scan_project_with_mode(root, mode)?)
}

#[allow(clippy::too_many_arguments)]
fn build_search_query(
    query_json: Option<String>,
    lang: Vec<String>,
    exclude_lang: Vec<String>,
    kind: Vec<String>,
    exclude_kind: Vec<String>,
    role: Vec<String>,
    exclude_role: Vec<String>,
    tech: Vec<String>,
    exclude_tech: Vec<String>,
    keyword: Vec<String>,
    exclude_keyword: Vec<String>,
    must: Vec<String>,
    any: Vec<String>,
    exclude: Vec<String>,
    prefer: Vec<String>,
    grep: Option<String>,
    regex: bool,
    case_sensitive: bool,
    context_before: usize,
    context_after: usize,
    max_matches_per_file: usize,
    limit: usize,
) -> Result<SearchQuery, Box<dyn Error>> {
    let has_structured_args = !lang.is_empty()
        || !exclude_lang.is_empty()
        || !kind.is_empty()
        || !exclude_kind.is_empty()
        || !role.is_empty()
        || !exclude_role.is_empty()
        || !tech.is_empty()
        || !exclude_tech.is_empty()
        || !keyword.is_empty()
        || !exclude_keyword.is_empty()
        || !must.is_empty()
        || !any.is_empty()
        || !exclude.is_empty()
        || !prefer.is_empty()
        || grep.is_some()
        || regex
        || case_sensitive
        || context_before > 0
        || context_after > 0
        || max_matches_per_file != 3
        || limit != 10;

    if let Some(query_json) = query_json {
        if has_structured_args {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "--query cannot be combined with --must/--any/--exclude/--prefer/--grep or grep options",
            )
            .into());
        }
        return Ok(serde_json::from_str::<SearchQuery>(&query_json)?);
    }

    let must = unique_tag_values(
        lang.into_iter()
            .map(|value| format!("lang:{value}"))
            .chain(kind.into_iter().map(|value| format!("kind:{value}")))
            .chain(role.into_iter().map(|value| format!("role:{value}")))
            .chain(tech.into_iter().map(|value| format!("tech:{value}")))
            .chain(keyword.into_iter().map(|value| format!("kw:{value}")))
            .chain(must)
            .collect::<Vec<_>>(),
    );
    let exclude = unique_tag_values(
        exclude_lang
            .into_iter()
            .map(|value| format!("lang:{value}"))
            .chain(
                exclude_kind
                    .into_iter()
                    .map(|value| format!("kind:{value}")),
            )
            .chain(
                exclude_role
                    .into_iter()
                    .map(|value| format!("role:{value}")),
            )
            .chain(
                exclude_tech
                    .into_iter()
                    .map(|value| format!("tech:{value}")),
            )
            .chain(
                exclude_keyword
                    .into_iter()
                    .map(|value| format!("kw:{value}")),
            )
            .chain(exclude)
            .collect::<Vec<_>>(),
    );

    Ok(SearchQuery {
        must,
        any,
        exclude,
        prefer,
        grep: grep.map(|pattern| GrepQuery {
            pattern,
            mode: if regex {
                GrepMode::Regex
            } else {
                GrepMode::Literal
            },
            case_sensitive,
            context_before,
            context_after,
            max_matches_per_file,
        }),
        limit,
    })
}

fn unique_tag_values(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn grep_search_query(
    pattern: String,
    regex: bool,
    case_sensitive: bool,
    context_before: usize,
    context_after: usize,
    max_matches_per_file: usize,
    limit: usize,
) -> SearchQuery {
    SearchQuery {
        must: vec![],
        any: vec![],
        exclude: vec![],
        prefer: vec![],
        grep: Some(GrepQuery {
            pattern,
            mode: if regex {
                GrepMode::Regex
            } else {
                GrepMode::Literal
            },
            case_sensitive,
            context_before,
            context_after,
            max_matches_per_file,
        }),
        limit,
    }
}

fn resolve_repo_root(root_hint: Option<PathBuf>) -> Result<PathBuf, Box<dyn Error>> {
    let start = root_hint.unwrap_or(std::env::current_dir()?);
    find_repo_root(&start).ok_or_else(|| {
        format!(
            "could not find a repository root containing .git from {}",
            start.display()
        )
        .into()
    })
}

fn find_repo_root(start: &Path) -> Option<PathBuf> {
    let start = fs::canonicalize(start).ok()?;
    let mut current = if start.is_file() {
        start.parent()?.to_path_buf()
    } else {
        start
    };

    loop {
        if current.join(".git").exists() {
            return Some(current);
        }
        current = current.parent()?.to_path_buf();
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{
        Cli, Command, SearchDisplay, SearchReasonDisplay, SearchSort, build_search_query,
        find_repo_root,
    };
    use crate::model::GrepMode;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("proj-finder-{name}-{nanos}"))
    }

    #[test]
    fn finds_git_root_from_nested_directory() {
        let root = temp_path("git-root");
        let nested = root.join("src").join("feature");
        fs::create_dir_all(root.join(".git")).expect("should create fake git dir");
        fs::create_dir_all(&nested).expect("should create nested dir");

        let found = find_repo_root(&nested).expect("should find repo root");
        assert_eq!(found, root);

        fs::remove_dir_all(found).expect("should clean up temp dir");
    }

    #[test]
    fn returns_none_when_git_root_is_missing() {
        let root = temp_path("no-git");
        let nested = root.join("src");
        fs::create_dir_all(&nested).expect("should create nested dir");

        assert!(find_repo_root(&nested).is_none());

        fs::remove_dir_all(root).expect("should clean up temp dir");
    }

    #[test]
    fn parses_grep_subcommand() {
        let cli = Cli::parse_from([
            "proj-finder",
            "grep",
            "ScanMode",
            ".",
            "--regex",
            "--context-before",
            "1",
            "--context-after",
            "2",
            "--max-matches-per-file",
            "5",
            "--limit",
            "7",
            "--incremental",
        ]);

        match cli.command {
            Some(Command::Grep {
                pattern,
                root,
                json,
                no_color,
                regex,
                case_sensitive,
                context_before,
                context_after,
                max_matches_per_file,
                limit,
                incremental,
            }) => {
                assert_eq!(pattern, "ScanMode");
                assert_eq!(root, Some(PathBuf::from(".")));
                assert!(!json);
                assert!(!no_color);
                assert!(regex);
                assert!(!case_sensitive);
                assert_eq!(context_before, 1);
                assert_eq!(context_after, 2);
                assert_eq!(max_matches_per_file, 5);
                assert_eq!(limit, 7);
                assert!(incremental);
            }
            other => panic!("expected grep command, got {other:?}"),
        }
    }

    #[test]
    fn parses_search_subcommand_with_human_friendly_flags() {
        let cli = Cli::parse_from([
            "proj-finder",
            "search",
            ".",
            "--lang",
            "rust",
            "--kind",
            "source",
            "--exclude-kind",
            "generated",
            "--role",
            "entrypoint",
            "--tech",
            "rust",
            "--keyword",
            "auth",
            "--exclude-role",
            "test",
            "--exclude-tech",
            "legacy",
            "--exclude-keyword",
            "generated",
            "--must",
            "lang:rust",
            "--must",
            "role:entrypoint",
            "--any",
            "kw:auth",
            "--exclude",
            "kind:generated",
            "--prefer",
            "dir:src",
            "--grep",
            "main",
            "--regex",
            "--case-sensitive",
            "--context-before",
            "1",
            "--context-after",
            "2",
            "--max-matches-per-file",
            "4",
            "--show",
            "full",
            "--sort",
            "path",
            "--show-reason",
            "brief",
            "--limit",
            "8",
            "--incremental",
        ]);

        match cli.command {
            Some(Command::Search {
                root,
                json,
                no_color,
                show,
                sort,
                show_reason,
                query,
                lang,
                exclude_lang,
                kind,
                exclude_kind,
                role,
                exclude_role,
                tech,
                exclude_tech,
                keyword,
                exclude_keyword,
                must,
                any,
                exclude,
                prefer,
                grep,
                regex,
                case_sensitive,
                context_before,
                context_after,
                max_matches_per_file,
                limit,
                incremental,
            }) => {
                assert_eq!(root, Some(PathBuf::from(".")));
                assert!(!json);
                assert!(!no_color);
                assert_eq!(show, SearchDisplay::Full);
                assert_eq!(sort, SearchSort::Path);
                assert_eq!(show_reason, SearchReasonDisplay::Brief);
                assert!(query.is_none());
                assert_eq!(lang, vec!["rust"]);
                assert!(exclude_lang.is_empty());
                assert_eq!(kind, vec!["source"]);
                assert_eq!(exclude_kind, vec!["generated"]);
                assert_eq!(role, vec!["entrypoint"]);
                assert_eq!(exclude_role, vec!["test"]);
                assert_eq!(tech, vec!["rust"]);
                assert_eq!(exclude_tech, vec!["legacy"]);
                assert_eq!(keyword, vec!["auth"]);
                assert_eq!(exclude_keyword, vec!["generated"]);
                assert_eq!(must, vec!["lang:rust", "role:entrypoint"]);
                assert_eq!(any, vec!["kw:auth"]);
                assert_eq!(exclude, vec!["kind:generated"]);
                assert_eq!(prefer, vec!["dir:src"]);
                assert_eq!(grep, Some("main".to_owned()));
                assert!(regex);
                assert!(case_sensitive);
                assert_eq!(context_before, 1);
                assert_eq!(context_after, 2);
                assert_eq!(max_matches_per_file, 4);
                assert_eq!(limit, 8);
                assert!(incremental);
            }
            other => panic!("expected search command, got {other:?}"),
        }
    }

    #[test]
    fn builds_search_query_from_human_friendly_flags() {
        let query = build_search_query(
            None,
            vec!["rust".to_owned()],
            vec!["markdown".to_owned()],
            vec!["source".to_owned()],
            vec!["generated".to_owned()],
            vec!["entrypoint".to_owned()],
            vec!["test".to_owned()],
            vec!["rust".to_owned()],
            vec!["legacy".to_owned()],
            vec!["auth".to_owned()],
            vec!["generated".to_owned()],
            vec!["lang:rust".to_owned()],
            vec!["kw:auth".to_owned()],
            vec!["kind:generated".to_owned()],
            vec!["dir:src".to_owned()],
            Some("main".to_owned()),
            true,
            true,
            1,
            2,
            4,
            8,
        )
        .expect("human-friendly search query should build");

        assert_eq!(
            query.must,
            vec![
                "lang:rust",
                "kind:source",
                "role:entrypoint",
                "tech:rust",
                "kw:auth"
            ]
        );
        assert_eq!(query.any, vec!["kw:auth"]);
        assert_eq!(
            query.exclude,
            vec![
                "lang:markdown",
                "kind:generated",
                "role:test",
                "tech:legacy",
                "kw:generated"
            ]
        );
        assert_eq!(query.prefer, vec!["dir:src"]);
        assert_eq!(
            query.grep.as_ref().expect("grep query should exist").mode,
            GrepMode::Regex
        );
        assert!(
            query
                .grep
                .as_ref()
                .expect("grep query should exist")
                .case_sensitive
        );
        assert_eq!(query.limit, 8);
    }

    #[test]
    fn rejects_mixing_json_query_with_human_friendly_flags() {
        let error = build_search_query(
            Some("{\"must\":[\"lang:rust\"]}".to_owned()),
            vec!["rust".to_owned()],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec!["role:entrypoint".to_owned()],
            vec![],
            vec![],
            vec![],
            None,
            false,
            false,
            0,
            0,
            3,
            10,
        )
        .expect_err("mixed query styles should fail");

        assert!(error.to_string().contains("--query cannot be combined"));
    }
}
