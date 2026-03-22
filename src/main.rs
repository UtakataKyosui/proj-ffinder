mod model;
mod polyglot_ast;
mod rust_ast;
mod scanner;
mod search;

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use model::SearchQuery;
use scanner::ScanMode;

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
        #[arg(long, value_name = "JSON")]
        query: String,
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
            query,
            incremental,
        }) => run_search(root, &query, incremental)?,
        None => run_scan(cli.root, false)?,
    }

    println!();
    Ok(())
}

fn run_scan(root: Option<PathBuf>, incremental: bool) -> Result<(), Box<dyn Error>> {
    let root = resolve_repo_root(root)?;
    let mode = if incremental {
        ScanMode::Incremental
    } else {
        ScanMode::Full
    };
    let scan_result = scanner::scan_project_with_mode(&root, mode)?;
    serde_json::to_writer_pretty(std::io::stdout(), &scan_result)?;
    Ok(())
}

fn run_search(
    root: Option<PathBuf>,
    query_json: &str,
    incremental: bool,
) -> Result<(), Box<dyn Error>> {
    let root = resolve_repo_root(root)?;
    let query = serde_json::from_str::<SearchQuery>(query_json)?;
    let mode = if incremental {
        ScanMode::Incremental
    } else {
        ScanMode::Full
    };
    let scan_result = scanner::scan_project_with_mode(&root, mode)?;
    let search_result = search::search_files(&scan_result.root, &scan_result.files, query);

    serde_json::to_writer_pretty(std::io::stdout(), &search_result)?;
    Ok(())
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
    use super::find_repo_root;
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
}
