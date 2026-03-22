use std::fmt::Write;

use regex::RegexBuilder;

use crate::model::{GrepMode, GrepQuery, MatchedTag, SearchResult};
use crate::{SearchDisplay, SearchReasonDisplay};

const COLOR_RESET: &str = "\x1b[0m";
const COLOR_PATH: &str = "\x1b[1;34m";
const COLOR_MATCH: &str = "\x1b[30;43m";
const COLOR_LINE_NUMBER: &str = "\x1b[2m";
const COLOR_LABEL: &str = "\x1b[35m";

pub fn format_search_result(
    result: &SearchResult,
    color: bool,
    display: SearchDisplay,
    reason_display: SearchReasonDisplay,
) -> Result<String, regex::Error> {
    let matcher = result.query.grep.as_ref().map(build_matcher).transpose()?;
    let mut out = String::new();

    if result.hits.is_empty() {
        writeln!(out, "No matches").expect("writing to string should succeed");
        return Ok(out);
    }

    for (hit_index, hit) in result.hits.iter().enumerate() {
        if hit_index > 0 {
            writeln!(out).expect("writing to string should succeed");
        }

        if display == SearchDisplay::Path {
            writeln!(out, "{}", colorize(COLOR_PATH, &hit.path, color))
                .expect("writing to string should succeed");
            continue;
        }

        writeln!(
            out,
            "{}  {} {:.1}",
            colorize(COLOR_PATH, &hit.path, color),
            colorize(COLOR_LABEL, "score", color),
            hit.score
        )
        .expect("writing to string should succeed");

        writeln!(
            out,
            "  {} {}",
            colorize(COLOR_LABEL, "type", color),
            format_hit_kind(hit.language.as_deref(), &hit.kind)
        )
        .expect("writing to string should succeed");

        if reason_display != SearchReasonDisplay::None && !hit.matched_must_details.is_empty() {
            writeln!(
                out,
                "  {} {}",
                colorize(COLOR_LABEL, "must", color),
                format_matched_tags(&hit.matched_must_details, reason_display)
            )
            .expect("writing to string should succeed");
        }

        if reason_display != SearchReasonDisplay::None && !hit.matched_any_details.is_empty() {
            writeln!(
                out,
                "  {} {}",
                colorize(COLOR_LABEL, "any", color),
                format_matched_tags(&hit.matched_any_details, reason_display)
            )
            .expect("writing to string should succeed");
        }

        if reason_display != SearchReasonDisplay::None && !hit.matched_prefer_details.is_empty() {
            writeln!(
                out,
                "  {} {}",
                colorize(COLOR_LABEL, "prefer", color),
                format_matched_tags(&hit.matched_prefer_details, reason_display)
            )
            .expect("writing to string should succeed");
        }

        if !hit.grep_matches.is_empty() && display == SearchDisplay::Summary {
            writeln!(
                out,
                "  {} {} match(es)",
                colorize(COLOR_LABEL, "grep", color),
                hit.grep_matches.len()
            )
            .expect("writing to string should succeed");
        }

        if let Some(matcher) = matcher.as_ref()
            && display == SearchDisplay::Full
        {
            for grep_match in &hit.grep_matches {
                for before in &grep_match.before {
                    writeln!(
                        out,
                        "{} {:>4} | {}",
                        colorize(COLOR_LINE_NUMBER, "-", color),
                        before.line_number,
                        before.line
                    )
                    .expect("writing to string should succeed");
                }

                writeln!(
                    out,
                    "{} {:>4} | {}",
                    colorize(COLOR_LINE_NUMBER, ">", color),
                    grep_match.line_number,
                    highlight_line(&grep_match.line, matcher, color)
                )
                .expect("writing to string should succeed");

                for after in &grep_match.after {
                    writeln!(
                        out,
                        "{} {:>4} | {}",
                        colorize(COLOR_LINE_NUMBER, "-", color),
                        after.line_number,
                        after.line
                    )
                    .expect("writing to string should succeed");
                }
            }
        }
    }

    if display != SearchDisplay::Path && !result.assistance.suggested_tags.is_empty() {
        writeln!(out).expect("writing to string should succeed");
        writeln!(out, "{}", colorize(COLOR_LABEL, "suggest", color))
            .expect("writing to string should succeed");
        for suggestion in result.assistance.suggested_tags.iter().take(3) {
            writeln!(
                out,
                "  {}  {} hit(s){}",
                suggestion.value,
                suggestion.hit_count,
                if suggestion.sample_paths.is_empty() {
                    String::new()
                } else {
                    format!("  [{}]", suggestion.sample_paths.join(", "))
                }
            )
            .expect("writing to string should succeed");
        }
    }

    if display != SearchDisplay::Path && !result.assistance.suggested_commands.is_empty() {
        writeln!(out).expect("writing to string should succeed");
        writeln!(out, "{}", colorize(COLOR_LABEL, "try", color))
            .expect("writing to string should succeed");
        for command in result.assistance.suggested_commands.iter().take(2) {
            writeln!(out, "  {}: {}", command.description, command.command)
                .expect("writing to string should succeed");
        }
    }

    Ok(out)
}

pub fn format_grep_result(result: &SearchResult, color: bool) -> Result<String, regex::Error> {
    let Some(grep) = result.query.grep.as_ref() else {
        return Ok(String::new());
    };
    let matcher = build_matcher(grep)?;
    let mut out = String::new();

    if result.hits.is_empty() {
        writeln!(out, "No matches").expect("writing to string should succeed");
        return Ok(out);
    }

    for (hit_index, hit) in result.hits.iter().enumerate() {
        if hit_index > 0 {
            writeln!(out).expect("writing to string should succeed");
        }

        writeln!(
            out,
            "{}{}",
            colorize(COLOR_PATH, &hit.path, color),
            if hit.language.is_some() || !hit.kind.is_empty() {
                format!(
                    "  [{}{}{}]",
                    hit.language.as_deref().unwrap_or("unknown"),
                    if hit.kind.is_empty() { "" } else { ":" },
                    hit.kind
                )
            } else {
                String::new()
            }
        )
        .expect("writing to string should succeed");

        for grep_match in &hit.grep_matches {
            for before in &grep_match.before {
                writeln!(
                    out,
                    "{} {:>4} | {}",
                    colorize(COLOR_LINE_NUMBER, "-", color),
                    before.line_number,
                    before.line
                )
                .expect("writing to string should succeed");
            }

            writeln!(
                out,
                "{} {:>4} | {}",
                colorize(COLOR_LINE_NUMBER, ">", color),
                grep_match.line_number,
                highlight_line(&grep_match.line, &matcher, color)
            )
            .expect("writing to string should succeed");

            for after in &grep_match.after {
                writeln!(
                    out,
                    "{} {:>4} | {}",
                    colorize(COLOR_LINE_NUMBER, "-", color),
                    after.line_number,
                    after.line
                )
                .expect("writing to string should succeed");
            }
        }
    }

    Ok(out)
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

fn format_hit_kind(language: Option<&str>, kind: &str) -> String {
    match (language, kind.is_empty()) {
        (Some(language), false) => format!("{language}:{kind}"),
        (Some(language), true) => language.to_owned(),
        (None, false) => kind.to_owned(),
        (None, true) => "unknown".to_owned(),
    }
}

fn format_matched_tags(tags: &[MatchedTag], reason_display: SearchReasonDisplay) -> String {
    tags.iter()
        .map(|tag| match reason_display {
            SearchReasonDisplay::None => tag.value.clone(),
            SearchReasonDisplay::Brief => tag.value.clone(),
            SearchReasonDisplay::Full => {
                if tag.evidence.is_empty() {
                    format!("{} ({}, {:.1})", tag.value, tag.source, tag.confidence)
                } else {
                    format!(
                        "{} ({}, {:.1}, {})",
                        tag.value,
                        tag.source,
                        tag.confidence,
                        tag.evidence.join("; ")
                    )
                }
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn highlight_line(line: &str, matcher: &regex::Regex, color: bool) -> String {
    if !color {
        return line.to_owned();
    }

    let mut highlighted = String::new();
    let mut cursor = 0;

    for found in matcher.find_iter(line) {
        highlighted.push_str(&line[cursor..found.start()]);
        highlighted.push_str(COLOR_MATCH);
        highlighted.push_str(found.as_str());
        highlighted.push_str(COLOR_RESET);
        cursor = found.end();
    }
    highlighted.push_str(&line[cursor..]);

    highlighted
}

fn colorize(color_code: &str, text: &str, color: bool) -> String {
    if color {
        format!("{color_code}{text}{COLOR_RESET}")
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{
        GrepContextLine, GrepMatch, GrepMode, GrepQuery, MatchedTag, SearchAssistance, SearchHit,
        SearchQuery, SearchResult, SuggestedCommand, SuggestedTag,
    };
    use crate::{SearchDisplay, SearchReasonDisplay};

    use super::{format_grep_result, format_search_result};

    fn sample_result() -> SearchResult {
        SearchResult {
            root: "/tmp/project".to_owned(),
            query: SearchQuery {
                must: vec![],
                any: vec![],
                exclude: vec![],
                prefer: vec![],
                grep: Some(GrepQuery {
                    pattern: "ScanMode".to_owned(),
                    mode: GrepMode::Literal,
                    case_sensitive: true,
                    context_before: 1,
                    context_after: 1,
                    max_matches_per_file: 3,
                }),
                limit: 10,
            },
            hits: vec![SearchHit {
                file_id: "src/main.rs".to_owned(),
                path: "src/main.rs".to_owned(),
                kind: "source".to_owned(),
                language: Some("rust".to_owned()),
                score: 50.0,
                matched_must: vec![],
                matched_must_details: vec![MatchedTag {
                    value: "lang:rust".to_owned(),
                    source: "path".to_owned(),
                    confidence: 1.0,
                    evidence: vec!["language=rust".to_owned()],
                }],
                matched_any: vec![],
                matched_any_details: vec![],
                matched_prefer: vec!["role:entrypoint".to_owned()],
                matched_prefer_details: vec![MatchedTag {
                    value: "role:entrypoint".to_owned(),
                    source: "content".to_owned(),
                    confidence: 0.8,
                    evidence: vec!["content-role=entrypoint".to_owned()],
                }],
                grep_matches: vec![GrepMatch {
                    line_number: 10,
                    line: "use scanner::ScanMode;".to_owned(),
                    before: vec![GrepContextLine {
                        line_number: 9,
                        line: "mod scanner;".to_owned(),
                    }],
                    after: vec![GrepContextLine {
                        line_number: 11,
                        line: "fn main() {}".to_owned(),
                    }],
                }],
            }],
            assistance: SearchAssistance {
                suggested_tags: vec![SuggestedTag {
                    value: "role:entrypoint".to_owned(),
                    hit_count: 1,
                    sample_paths: vec!["src/main.rs".to_owned()],
                }],
                suggested_commands: vec![SuggestedCommand {
                    description: "narrow with role:entrypoint".to_owned(),
                    command: "proj-finder search . --must role:entrypoint --limit 10".to_owned(),
                }],
            },
        }
    }

    #[test]
    fn formats_grep_result_without_color() {
        let formatted = format_grep_result(&sample_result(), false)
            .expect("formatting grep result should succeed");

        assert!(formatted.contains("src/main.rs  [rust:source]"));
        assert!(formatted.contains(">   10 | use scanner::ScanMode;"));
        assert!(formatted.contains("-    9 | mod scanner;"));
        assert!(formatted.contains("-   11 | fn main() {}"));
        assert!(!formatted.contains("\x1b["));
    }

    #[test]
    fn formats_grep_result_with_highlight() {
        let formatted = format_grep_result(&sample_result(), true)
            .expect("formatting grep result should succeed");

        assert!(formatted.contains("\x1b[1;34msrc/main.rs\x1b[0m"));
        assert!(formatted.contains("\x1b[30;43mScanMode\x1b[0m"));
    }

    #[test]
    fn formats_search_result_without_color() {
        let formatted = format_search_result(
            &sample_result(),
            false,
            SearchDisplay::Full,
            SearchReasonDisplay::Full,
        )
        .expect("formatting search result should succeed");

        assert!(formatted.contains("src/main.rs  score 50.0"));
        assert!(formatted.contains("type rust:source"));
        assert!(formatted.contains("must lang:rust (path, 1.0, language=rust)"));
        assert!(
            formatted.contains("prefer role:entrypoint (content, 0.8, content-role=entrypoint)")
        );
        assert!(formatted.contains(">   10 | use scanner::ScanMode;"));
        assert!(formatted.contains("suggest"));
        assert!(formatted.contains("try"));
        assert!(!formatted.contains("\x1b["));
    }

    #[test]
    fn formats_search_result_with_color() {
        let formatted = format_search_result(
            &sample_result(),
            true,
            SearchDisplay::Full,
            SearchReasonDisplay::Full,
        )
        .expect("formatting search result should succeed");

        assert!(formatted.contains("\x1b[1;34msrc/main.rs\x1b[0m"));
        assert!(formatted.contains("\x1b[35mscore\x1b[0m"));
        assert!(formatted.contains("\x1b[30;43mScanMode\x1b[0m"));
    }

    #[test]
    fn formats_search_result_in_summary_mode() {
        let formatted = format_search_result(
            &sample_result(),
            false,
            SearchDisplay::Summary,
            SearchReasonDisplay::Full,
        )
        .expect("formatting search result should succeed");

        assert!(formatted.contains("src/main.rs  score 50.0"));
        assert!(formatted.contains("grep 1 match(es)"));
        assert!(!formatted.contains(">   10 | use scanner::ScanMode;"));
    }

    #[test]
    fn formats_search_result_in_path_mode() {
        let formatted = format_search_result(
            &sample_result(),
            false,
            SearchDisplay::Path,
            SearchReasonDisplay::Full,
        )
        .expect("formatting search result should succeed");

        assert_eq!(formatted, "src/main.rs\n");
    }

    #[test]
    fn formats_search_result_with_brief_reason_mode() {
        let formatted = format_search_result(
            &sample_result(),
            false,
            SearchDisplay::Summary,
            SearchReasonDisplay::Brief,
        )
        .expect("formatting search result should succeed");

        assert!(formatted.contains("must lang:rust"));
        assert!(!formatted.contains("language=rust"));
    }

    #[test]
    fn formats_search_result_with_reason_hidden() {
        let formatted = format_search_result(
            &sample_result(),
            false,
            SearchDisplay::Summary,
            SearchReasonDisplay::None,
        )
        .expect("formatting search result should succeed");

        assert!(!formatted.contains("\n  must "));
        assert!(!formatted.contains("\n  prefer "));
    }
}
