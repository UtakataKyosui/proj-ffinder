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
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use log::{Level, LevelFilter, Metadata, Record};
use model::{GrepMode, GrepQuery, SearchQuery};
use scanner::ScanMode;

struct StderrLogger;

static LOGGER: StderrLogger = StderrLogger;
static LOGGER_INIT: OnceLock<()> = OnceLock::new();

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let _ = writeln!(
            io::stderr(),
            "{}: {}",
            level_prefix(record.level()),
            record.args()
        );
    }

    fn flush(&self) {}
}

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

    /// Repository root hint. If omitted, the current directory is used and
    /// `.git` is searched upward to find the repository root.
    #[arg(value_name = "ROOT")]
    root: Option<PathBuf>,
}

#[derive(Clone, Debug, Args)]
struct SearchTagArgs {
    /// Require files tagged with this language, for example `rust`.
    #[arg(long, value_name = "LANG", action = ArgAction::Append)]
    lang: Vec<String>,
    /// Exclude files tagged with this language.
    #[arg(long, value_name = "LANG", action = ArgAction::Append)]
    exclude_lang: Vec<String>,
    /// Require files tagged with this kind, for example `source` or `docs`.
    #[arg(long, value_name = "KIND", action = ArgAction::Append)]
    kind: Vec<String>,
    /// Exclude files tagged with this kind.
    #[arg(long, value_name = "KIND", action = ArgAction::Append)]
    exclude_kind: Vec<String>,
    /// Require files tagged with this role, for example `entrypoint` or `test`.
    #[arg(long, value_name = "ROLE", action = ArgAction::Append)]
    role: Vec<String>,
    /// Exclude files tagged with this role.
    #[arg(long, value_name = "ROLE", action = ArgAction::Append)]
    exclude_role: Vec<String>,
    /// Require files tagged with this technology, for example `react`.
    #[arg(long, value_name = "TECH", action = ArgAction::Append)]
    tech: Vec<String>,
    /// Exclude files tagged with this technology.
    #[arg(long, value_name = "TECH", action = ArgAction::Append)]
    exclude_tech: Vec<String>,
    /// Require files tagged with this extracted keyword.
    #[arg(long, value_name = "KEYWORD", action = ArgAction::Append)]
    keyword: Vec<String>,
    /// Exclude files tagged with this extracted keyword.
    #[arg(long, value_name = "KEYWORD", action = ArgAction::Append)]
    exclude_keyword: Vec<String>,
    /// Require this exact tag value, for example `lang:rust`.
    #[arg(long, value_name = "TAG", action = ArgAction::Append)]
    must: Vec<String>,
    /// Require at least one of these exact tag values.
    #[arg(long, value_name = "TAG", action = ArgAction::Append)]
    any: Vec<String>,
    /// Exclude files matching any of these exact tag values.
    #[arg(long, value_name = "TAG", action = ArgAction::Append)]
    exclude: Vec<String>,
    /// Boost files matching these exact tag values.
    #[arg(long, value_name = "TAG", action = ArgAction::Append)]
    prefer: Vec<String>,
}

impl SearchTagArgs {
    fn has_values(&self) -> bool {
        !self.lang.is_empty()
            || !self.exclude_lang.is_empty()
            || !self.kind.is_empty()
            || !self.exclude_kind.is_empty()
            || !self.role.is_empty()
            || !self.exclude_role.is_empty()
            || !self.tech.is_empty()
            || !self.exclude_tech.is_empty()
            || !self.keyword.is_empty()
            || !self.exclude_keyword.is_empty()
            || !self.must.is_empty()
            || !self.any.is_empty()
            || !self.exclude.is_empty()
            || !self.prefer.is_empty()
    }
}

#[derive(Clone, Debug, Args)]
struct GrepOptionArgs {
    /// Treat the grep pattern as a regular expression instead of a literal string.
    #[arg(long)]
    regex: bool,
    /// Make grep matching case-sensitive.
    #[arg(long)]
    case_sensitive: bool,
    /// Show this many lines before each grep match.
    #[arg(long, default_value_t = 0)]
    context_before: usize,
    /// Show this many lines after each grep match.
    #[arg(long, default_value_t = 0)]
    context_after: usize,
    /// Stop reading a file after this many grep matches were collected.
    #[arg(long, default_value_t = 3)]
    max_matches_per_file: usize,
}

impl GrepOptionArgs {
    fn has_custom_values(&self) -> bool {
        self.regex
            || self.case_sensitive
            || self.context_before > 0
            || self.context_after > 0
            || self.max_matches_per_file != 3
    }
}

#[derive(Clone, Debug, Args)]
struct SearchCliArgs {
    /// Print JSON instead of human-friendly formatted output.
    #[arg(long)]
    json: bool,
    /// Disable ANSI colors in formatted output.
    #[arg(long)]
    no_color: bool,
    /// Choose how much information to show for each hit.
    #[arg(long, value_enum, default_value_t = SearchDisplay::Summary)]
    show: SearchDisplay,
    /// Sort hits by score or by path before applying `--limit`.
    #[arg(long, value_enum, default_value_t = SearchSort::Score)]
    sort: SearchSort,
    /// Control how much tag-match reasoning to display.
    #[arg(long, value_enum, default_value_t = SearchReasonDisplay::Full)]
    show_reason: SearchReasonDisplay,
    /// Provide the full query as JSON instead of using structured flags.
    #[arg(long, value_name = "JSON")]
    query: Option<String>,
    #[command(flatten)]
    tags: SearchTagArgs,
    /// Restrict matches to files whose contents match this pattern.
    #[arg(long, value_name = "PATTERN")]
    grep: Option<String>,
    #[command(flatten)]
    grep_options: GrepOptionArgs,
    /// Maximum number of hits to print after sorting.
    #[arg(long, default_value_t = 10)]
    limit: usize,
    /// Refresh the on-disk cache incrementally before searching.
    #[arg(long)]
    incremental: bool,
}

#[derive(Clone, Debug, Args)]
struct GrepCliArgs {
    /// Print JSON instead of human-friendly formatted output.
    #[arg(long)]
    json: bool,
    /// Disable ANSI colors in formatted output.
    #[arg(long)]
    no_color: bool,
    #[command(flatten)]
    grep_options: GrepOptionArgs,
    /// Maximum number of files to print after sorting by score.
    #[arg(long, default_value_t = 10)]
    limit: usize,
    /// Refresh the on-disk cache incrementally before searching.
    #[arg(long)]
    incremental: bool,
}

#[derive(Debug, Subcommand)]
#[allow(clippy::large_enum_variant)]
enum Command {
    /// Scan the repository and emit file summaries as JSON.
    Scan {
        /// Repository root hint. `.git` is searched upward from this path.
        #[arg(value_name = "ROOT")]
        root: Option<PathBuf>,
        /// Refresh the on-disk cache incrementally when possible.
        #[arg(long)]
        incremental: bool,
    },
    /// Search indexed files by tags and optional grep pattern.
    Search {
        /// Repository root hint. `.git` is searched upward from this path.
        #[arg(value_name = "ROOT")]
        root: Option<PathBuf>,
        #[command(flatten)]
        args: SearchCliArgs,
    },
    /// Search file contents like grep and print matching lines.
    Grep {
        /// Literal text or regular expression to search for.
        #[arg(value_name = "PATTERN")]
        pattern: String,
        /// Repository root hint. `.git` is searched upward from this path.
        #[arg(value_name = "ROOT")]
        root: Option<PathBuf>,
        #[command(flatten)]
        args: GrepCliArgs,
    },
}

fn main() -> Result<(), Box<dyn Error>> {
    init_logging();
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Scan { root, incremental }) => run_scan(root, incremental)?,
        Some(Command::Search { root, args }) => run_search(root, &args)?,
        Some(Command::Grep {
            pattern,
            root,
            args,
        }) => run_grep(root, &pattern, &args)?,
        None => run_scan(cli.root, false)?,
    }

    println!();
    Ok(())
}

fn init_logging() {
    LOGGER_INIT.get_or_init(|| {
        let level = std::env::var("PROJ_FINDER_LOG")
            .ok()
            .and_then(|value| parse_log_level(&value))
            .unwrap_or(LevelFilter::Warn);
        let _ = log::set_logger(&LOGGER).map(|()| log::set_max_level(level));
    });
}

fn parse_log_level(value: &str) -> Option<LevelFilter> {
    match value.to_ascii_lowercase().as_str() {
        "off" => Some(LevelFilter::Off),
        "error" => Some(LevelFilter::Error),
        "warn" | "warning" => Some(LevelFilter::Warn),
        "info" => Some(LevelFilter::Info),
        "debug" => Some(LevelFilter::Debug),
        "trace" => Some(LevelFilter::Trace),
        _ => None,
    }
}

fn level_prefix(level: Level) -> &'static str {
    match level {
        Level::Error => "error",
        Level::Warn => "warn",
        Level::Info => "info",
        Level::Debug => "debug",
        Level::Trace => "trace",
    }
}

fn run_scan(root: Option<PathBuf>, incremental: bool) -> Result<(), Box<dyn Error>> {
    let root = resolve_repo_root(root)?;
    let scan_result = scan_result_for(&root, incremental)?;
    serde_json::to_writer_pretty(std::io::stdout(), &scan_result)?;
    Ok(())
}

fn run_search(root: Option<PathBuf>, args: &SearchCliArgs) -> Result<(), Box<dyn Error>> {
    let root = resolve_repo_root(root)?;
    let query = build_search_query(args)?;
    let mut search_result = if args.incremental {
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
    finalize_search_result(&mut search_result, args.sort);

    if args.json {
        serde_json::to_writer_pretty(std::io::stdout(), &search_result)?;
    } else {
        print!(
            "{}",
            formatter::format_search_result(
                &search_result,
                !args.no_color,
                args.show,
                args.show_reason,
            )?
        );
    }
    Ok(())
}

fn run_grep(
    root: Option<PathBuf>,
    pattern: &str,
    args: &GrepCliArgs,
) -> Result<(), Box<dyn Error>> {
    let root = resolve_repo_root(root)?;
    let query = grep_search_query(
        pattern.to_owned(),
        args.grep_options.regex,
        args.grep_options.case_sensitive,
        args.grep_options.context_before,
        args.grep_options.context_after,
        args.grep_options.max_matches_per_file,
        args.limit,
    );
    let mut search_result = if args.incremental {
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

    if args.json {
        serde_json::to_writer_pretty(std::io::stdout(), &search_result)?;
    } else {
        print!(
            "{}",
            formatter::format_grep_result(&search_result, !args.no_color)?
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

// Build a structured search query from the human-friendly CLI flags while
// preserving the older `--query <json>` escape hatch.
fn build_search_query(args: &SearchCliArgs) -> Result<SearchQuery, Box<dyn Error>> {
    let has_structured_args = args.tags.has_values()
        || args.grep.is_some()
        || args.grep_options.has_custom_values()
        || args.limit != 10;

    if let Some(query_json) = args.query.as_deref() {
        if has_structured_args {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "--query cannot be combined with structured search flags (such as --lang/--kind/--role/--tech/--keyword and their --exclude-* variants, --must/--any/--exclude/--prefer, --grep and grep context/max-match options, or --limit)",
            )
            .into());
        }
        return Ok(serde_json::from_str::<SearchQuery>(query_json)?);
    }

    let must = unique_tag_values(
        prefix_tags(&args.tags.lang, "lang")
            .chain(prefix_tags(&args.tags.kind, "kind"))
            .chain(prefix_tags(&args.tags.role, "role"))
            .chain(prefix_tags(&args.tags.tech, "tech"))
            .chain(prefix_tags(&args.tags.keyword, "kw"))
            .chain(args.tags.must.iter().cloned())
            .collect::<Vec<_>>(),
    );
    let exclude = unique_tag_values(
        prefix_tags(&args.tags.exclude_lang, "lang")
            .chain(prefix_tags(&args.tags.exclude_kind, "kind"))
            .chain(prefix_tags(&args.tags.exclude_role, "role"))
            .chain(prefix_tags(&args.tags.exclude_tech, "tech"))
            .chain(prefix_tags(&args.tags.exclude_keyword, "kw"))
            .chain(args.tags.exclude.iter().cloned())
            .collect::<Vec<_>>(),
    );

    Ok(SearchQuery {
        must,
        any: args.tags.any.clone(),
        exclude,
        prefer: args.tags.prefer.clone(),
        grep: args.grep.clone().map(|pattern| GrepQuery {
            pattern,
            mode: if args.grep_options.regex {
                GrepMode::Regex
            } else {
                GrepMode::Literal
            },
            case_sensitive: args.grep_options.case_sensitive,
            context_before: args.grep_options.context_before,
            context_after: args.grep_options.context_after,
            max_matches_per_file: args.grep_options.max_matches_per_file,
        }),
        limit: args.limit,
    })
}

fn unique_tag_values(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

// Expand shorthand flags such as `--lang rust` into exact tag values like
// `lang:rust` so the search engine only needs to reason about tags.
fn prefix_tags<'a>(values: &'a [String], prefix: &'a str) -> impl Iterator<Item = String> + 'a {
    values.iter().map(move |value| format!("{prefix}:{value}"))
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
        Cli, Command, GrepOptionArgs, SearchCliArgs, SearchDisplay, SearchReasonDisplay,
        SearchSort, SearchTagArgs, build_search_query, find_repo_root,
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
                args,
            }) => {
                assert_eq!(pattern, "ScanMode");
                assert_eq!(root, Some(PathBuf::from(".")));
                assert!(!args.json);
                assert!(!args.no_color);
                assert!(args.grep_options.regex);
                assert!(!args.grep_options.case_sensitive);
                assert_eq!(args.grep_options.context_before, 1);
                assert_eq!(args.grep_options.context_after, 2);
                assert_eq!(args.grep_options.max_matches_per_file, 5);
                assert_eq!(args.limit, 7);
                assert!(args.incremental);
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
            Some(Command::Search { root, args }) => {
                assert_eq!(root, Some(PathBuf::from(".")));
                assert!(!args.json);
                assert!(!args.no_color);
                assert_eq!(args.show, SearchDisplay::Full);
                assert_eq!(args.sort, SearchSort::Path);
                assert_eq!(args.show_reason, SearchReasonDisplay::Brief);
                assert!(args.query.is_none());
                assert_eq!(args.tags.lang, vec!["rust"]);
                assert!(args.tags.exclude_lang.is_empty());
                assert_eq!(args.tags.kind, vec!["source"]);
                assert_eq!(args.tags.exclude_kind, vec!["generated"]);
                assert_eq!(args.tags.role, vec!["entrypoint"]);
                assert_eq!(args.tags.exclude_role, vec!["test"]);
                assert_eq!(args.tags.tech, vec!["rust"]);
                assert_eq!(args.tags.exclude_tech, vec!["legacy"]);
                assert_eq!(args.tags.keyword, vec!["auth"]);
                assert_eq!(args.tags.exclude_keyword, vec!["generated"]);
                assert_eq!(args.tags.must, vec!["lang:rust", "role:entrypoint"]);
                assert_eq!(args.tags.any, vec!["kw:auth"]);
                assert_eq!(args.tags.exclude, vec!["kind:generated"]);
                assert_eq!(args.tags.prefer, vec!["dir:src"]);
                assert_eq!(args.grep, Some("main".to_owned()));
                assert!(args.grep_options.regex);
                assert!(args.grep_options.case_sensitive);
                assert_eq!(args.grep_options.context_before, 1);
                assert_eq!(args.grep_options.context_after, 2);
                assert_eq!(args.grep_options.max_matches_per_file, 4);
                assert_eq!(args.limit, 8);
                assert!(args.incremental);
            }
            other => panic!("expected search command, got {other:?}"),
        }
    }

    #[test]
    fn builds_search_query_from_human_friendly_flags() {
        let query = build_search_query(&SearchCliArgs {
            json: false,
            no_color: false,
            show: SearchDisplay::Summary,
            sort: SearchSort::Score,
            show_reason: SearchReasonDisplay::Full,
            query: None,
            tags: SearchTagArgs {
                lang: vec!["rust".to_owned()],
                exclude_lang: vec!["markdown".to_owned()],
                kind: vec!["source".to_owned()],
                exclude_kind: vec!["generated".to_owned()],
                role: vec!["entrypoint".to_owned()],
                exclude_role: vec!["test".to_owned()],
                tech: vec!["rust".to_owned()],
                exclude_tech: vec!["legacy".to_owned()],
                keyword: vec!["auth".to_owned()],
                exclude_keyword: vec!["generated".to_owned()],
                must: vec!["lang:rust".to_owned()],
                any: vec!["kw:auth".to_owned()],
                exclude: vec!["kind:generated".to_owned()],
                prefer: vec!["dir:src".to_owned()],
            },
            grep: Some("main".to_owned()),
            grep_options: GrepOptionArgs {
                regex: true,
                case_sensitive: true,
                context_before: 1,
                context_after: 2,
                max_matches_per_file: 4,
            },
            limit: 8,
            incremental: false,
        })
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
        let error = build_search_query(&SearchCliArgs {
            json: false,
            no_color: false,
            show: SearchDisplay::Summary,
            sort: SearchSort::Score,
            show_reason: SearchReasonDisplay::Full,
            query: Some("{\"must\":[\"lang:rust\"]}".to_owned()),
            tags: SearchTagArgs {
                lang: vec!["rust".to_owned()],
                exclude_lang: vec![],
                kind: vec![],
                exclude_kind: vec![],
                role: vec![],
                exclude_role: vec![],
                tech: vec![],
                exclude_tech: vec![],
                keyword: vec![],
                exclude_keyword: vec![],
                must: vec!["role:entrypoint".to_owned()],
                any: vec![],
                exclude: vec![],
                prefer: vec![],
            },
            grep: None,
            grep_options: GrepOptionArgs {
                regex: false,
                case_sensitive: false,
                context_before: 0,
                context_after: 0,
                max_matches_per_file: 3,
            },
            limit: 10,
            incremental: false,
        })
        .expect_err("mixed query styles should fail");

        assert!(error.to_string().contains("structured search flags"));
    }
}
