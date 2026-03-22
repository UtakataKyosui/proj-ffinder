use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use bincode::config::standard;
use bincode::serde::{decode_from_slice, encode_to_vec};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};

use crate::model::{ContentMetrics, ContentSummary, FileSummary, LocationSummary, ScanResult, Tag};
use crate::polyglot_ast::analyze_other_file;
use crate::rust_ast::analyze_rust_file;

const INDEX_DIR: &str = ".proj-finder";
const FILES_INDEX_FILE: &str = "files.bin";
const TAGS_INDEX_FILE: &str = "tags.bin";
const LEGACY_COMBINED_INDEX_FILE: &str = "index.bin";
const LEGACY_JSON_INDEX_FILE: &str = "index.json";
const INDEX_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ScanMode {
    Full,
    Incremental,
}

#[derive(Debug, Deserialize, Serialize)]
struct PersistedFilesIndex {
    version: u32,
    root: String,
    files: Vec<PersistedFileSummary>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CachedFileSummary {
    path: String,
    hash: String,
    summary: FileSummary,
}

#[derive(Debug)]
struct PersistedIndex {
    version: u32,
    root: String,
    files: Vec<CachedFileSummary>,
}

#[derive(Debug, Deserialize, Serialize)]
struct PersistedTagsIndex {
    version: u32,
    root: String,
    strings: Vec<String>,
    file_tags: Vec<Vec<CompactTag>>,
    inverted_index: Vec<CompactPosting>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct PersistedFileSummary {
    path: String,
    hash: String,
    language: Option<String>,
    kind: String,
    location_summary: LocationSummary,
    content_summary: Option<ContentSummary>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CompactTag {
    value_id: u32,
    source_id: u32,
    confidence: f32,
    evidence_ids: Vec<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CompactPosting {
    value_id: u32,
    file_indices: Vec<u32>,
}

#[derive(Debug, Default)]
struct StringTableBuilder {
    strings: Vec<String>,
    ids: HashMap<String, u32>,
}

pub fn scan_project(root: &Path) -> io::Result<ScanResult> {
    scan_project_with_mode(root, ScanMode::Full)
}

pub fn scan_project_with_mode(root: &Path, mode: ScanMode) -> io::Result<ScanResult> {
    let root = root.canonicalize()?;
    match mode {
        ScanMode::Full => full_scan(&root),
        ScanMode::Incremental => incremental_scan(&root),
    }
}

fn full_scan(root: &Path) -> io::Result<ScanResult> {
    let mut file_paths = collect_scannable_files(root)?;
    file_paths.sort();

    let cached_files = file_paths
        .into_iter()
        .map(|path| cached_summary_for_path(root, &path))
        .collect::<io::Result<Vec<_>>>()?;
    persist_index(root, &cached_files)?;

    Ok(ScanResult {
        root: root.display().to_string(),
        files: cached_files
            .into_iter()
            .map(|cached| cached.summary)
            .collect::<Vec<_>>(),
    })
}

fn incremental_scan(root: &Path) -> io::Result<ScanResult> {
    let Some(index) = load_index(root)? else {
        return full_scan(root);
    };
    if index.version != INDEX_VERSION || index.root != root.display().to_string() {
        return full_scan(root);
    }

    let mut current_paths = collect_scannable_files(root)?;
    current_paths.sort();
    let current_map = current_paths
        .into_iter()
        .map(|absolute_path| {
            let relative = absolute_path
                .strip_prefix(root)
                .expect("scanned file should stay under root");
            (normalize_path(relative), absolute_path)
        })
        .collect::<HashMap<_, _>>();

    let cached_map = index
        .files
        .into_iter()
        .map(|cached| (cached.path.clone(), cached))
        .collect::<HashMap<_, _>>();

    let changed_paths = match collect_git_changed_paths(root) {
        Ok(changed)
            if !changed.contains(".gitignore")
                && !changed.iter().any(|path| path.starts_with(".gitignore/")) =>
        {
            changed
        }
        _ => collect_changed_paths_by_hash(&current_map, &cached_map)?,
    };

    let mut ordered_paths = current_map.keys().cloned().collect::<Vec<_>>();
    ordered_paths.sort();

    let mut cached_files = Vec::new();
    for path in ordered_paths {
        if !changed_paths.contains(&path)
            && let Some(cached) = cached_map.get(&path)
        {
            cached_files.push(cached.clone());
            continue;
        }

        let absolute_path = current_map
            .get(&path)
            .expect("ordered path should resolve to a current file");
        cached_files.push(cached_summary_for_path(root, absolute_path)?);
    }

    persist_index(root, &cached_files)?;

    Ok(ScanResult {
        root: root.display().to_string(),
        files: cached_files
            .into_iter()
            .map(|cached| cached.summary)
            .collect::<Vec<_>>(),
    })
}

fn collect_scannable_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut builder = WalkBuilder::new(root);
    builder.hidden(false);
    builder.git_ignore(true);
    builder.git_global(false);
    builder.git_exclude(false);
    builder.parents(true);

    let mut files = Vec::new();
    for entry in builder.build() {
        let entry = entry.map_err(io::Error::other)?;
        if path_contains_any_component(entry.path(), &[".git", INDEX_DIR]) {
            continue;
        }
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        files.push(entry.into_path());
    }

    Ok(files)
}

fn cached_summary_for_path(root: &Path, absolute_path: &Path) -> io::Result<CachedFileSummary> {
    let relative = absolute_path
        .strip_prefix(root)
        .expect("scanned file should stay under root");
    let path = normalize_path(relative);
    let summary = summarize_file(absolute_path, relative);
    let hash = compute_file_hash(absolute_path)?;

    Ok(CachedFileSummary {
        path,
        hash,
        summary,
    })
}

fn load_index(root: &Path) -> io::Result<Option<PersistedIndex>> {
    let Some(files_index) = load_files_index(root)? else {
        return Ok(None);
    };
    let Some(tags_index) = load_tags_index(root)? else {
        return Ok(None);
    };

    if files_index.version != INDEX_VERSION
        || tags_index.version != INDEX_VERSION
        || files_index.root != root.display().to_string()
        || tags_index.root != root.display().to_string()
        || files_index.files.len() != tags_index.file_tags.len()
    {
        return Ok(None);
    }

    let mut files = Vec::with_capacity(files_index.files.len());
    for (file, tags) in files_index.files.into_iter().zip(tags_index.file_tags) {
        let Some(decoded_tags) = decode_tags(&tags, &tags_index.strings) else {
            return Ok(None);
        };
        files.push(CachedFileSummary {
            path: file.path.clone(),
            hash: file.hash,
            summary: FileSummary {
                file_id: file.path.clone(),
                path: file.path,
                language: file.language,
                kind: file.kind,
                location_summary: file.location_summary,
                content_summary: file.content_summary,
                tags: decoded_tags,
            },
        });
    }

    Ok(Some(PersistedIndex {
        version: INDEX_VERSION,
        root: root.display().to_string(),
        files,
    }))
}

fn persist_index(root: &Path, files: &[CachedFileSummary]) -> io::Result<()> {
    let index_dir = root.join(INDEX_DIR);
    fs::create_dir_all(&index_dir)?;

    let files_payload = PersistedFilesIndex {
        version: INDEX_VERSION,
        root: root.display().to_string(),
        files: files
            .iter()
            .map(|file| PersistedFileSummary {
                path: file.path.clone(),
                hash: file.hash.clone(),
                language: file.summary.language.clone(),
                kind: file.summary.kind.clone(),
                location_summary: file.summary.location_summary.clone(),
                content_summary: file.summary.content_summary.clone(),
            })
            .collect(),
    };
    let tags_payload = build_tags_index(root, files);

    let files_bytes = encode_to_vec(&files_payload, standard())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let tags_bytes = encode_to_vec(&tags_payload, standard())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    fs::write(files_index_path(root), files_bytes)?;
    fs::write(tags_index_path(root), tags_bytes)?;

    for legacy_file in [LEGACY_COMBINED_INDEX_FILE, LEGACY_JSON_INDEX_FILE] {
        let legacy_path = root.join(INDEX_DIR).join(legacy_file);
        if legacy_path.exists() {
            fs::remove_file(legacy_path)?;
        }
    }

    Ok(())
}

fn load_files_index(root: &Path) -> io::Result<Option<PersistedFilesIndex>> {
    let path = files_index_path(root);
    if !path.exists() {
        return Ok(None);
    }

    let bytes = fs::read(path)?;
    match decode_from_slice::<PersistedFilesIndex, _>(&bytes, standard()) {
        Ok((index, _)) => Ok(Some(index)),
        Err(_) => Ok(None),
    }
}

fn load_tags_index(root: &Path) -> io::Result<Option<PersistedTagsIndex>> {
    let path = tags_index_path(root);
    if !path.exists() {
        return Ok(None);
    }

    let bytes = fs::read(path)?;
    match decode_from_slice::<PersistedTagsIndex, _>(&bytes, standard()) {
        Ok((index, _)) => Ok(Some(index)),
        Err(_) => Ok(None),
    }
}

fn build_tags_index(root: &Path, files: &[CachedFileSummary]) -> PersistedTagsIndex {
    let mut strings = StringTableBuilder::default();
    let mut file_tags = Vec::with_capacity(files.len());
    let mut postings = HashMap::<u32, Vec<u32>>::new();

    for (file_index, file) in files.iter().enumerate() {
        let mut compact_tags = Vec::with_capacity(file.summary.tags.len());
        for tag in &file.summary.tags {
            let value_id = strings.intern(&tag.value);
            let source_id = strings.intern(&tag.source);
            let evidence_ids = tag
                .evidence
                .iter()
                .map(|evidence| strings.intern(evidence))
                .collect::<Vec<_>>();

            compact_tags.push(CompactTag {
                value_id,
                source_id,
                confidence: tag.confidence,
                evidence_ids,
            });
            postings
                .entry(value_id)
                .or_default()
                .push(file_index as u32);
        }
        file_tags.push(compact_tags);
    }

    let mut inverted_index = postings
        .into_iter()
        .map(|(value_id, file_indices)| CompactPosting {
            value_id,
            file_indices,
        })
        .collect::<Vec<_>>();
    inverted_index.sort_by_key(|posting| posting.value_id);

    PersistedTagsIndex {
        version: INDEX_VERSION,
        root: root.display().to_string(),
        strings: strings.strings,
        file_tags,
        inverted_index,
    }
}

fn decode_tags(tags: &[CompactTag], strings: &[String]) -> Option<Vec<Tag>> {
    tags.iter()
        .map(|tag| {
            Some(Tag {
                value: strings.get(tag.value_id as usize)?.clone(),
                source: strings.get(tag.source_id as usize)?.clone(),
                confidence: tag.confidence,
                evidence: tag
                    .evidence_ids
                    .iter()
                    .map(|id| strings.get(*id as usize).cloned())
                    .collect::<Option<Vec<_>>>()?,
            })
        })
        .collect()
}

fn files_index_path(root: &Path) -> PathBuf {
    root.join(INDEX_DIR).join(FILES_INDEX_FILE)
}

fn tags_index_path(root: &Path) -> PathBuf {
    root.join(INDEX_DIR).join(TAGS_INDEX_FILE)
}

impl StringTableBuilder {
    fn intern(&mut self, value: &str) -> u32 {
        if let Some(id) = self.ids.get(value) {
            return *id;
        }
        let id = self.strings.len() as u32;
        let owned = value.to_owned();
        self.strings.push(owned.clone());
        self.ids.insert(owned, id);
        id
    }
}

fn collect_git_changed_paths(root: &Path) -> io::Result<HashSet<String>> {
    let mut changed = HashSet::new();
    for args in [
        &["diff", "--name-only", "--relative"][..],
        &["diff", "--name-only", "--relative", "--cached"][..],
        &["ls-files", "--others", "--exclude-standard"][..],
    ] {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(String::from_utf8_lossy(&output.stderr)));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            changed.insert(normalize_path(Path::new(line)));
        }
    }

    Ok(changed)
}

fn collect_changed_paths_by_hash(
    current_map: &HashMap<String, PathBuf>,
    cached_map: &HashMap<String, CachedFileSummary>,
) -> io::Result<HashSet<String>> {
    let mut changed = HashSet::new();

    for (path, absolute_path) in current_map {
        let hash = compute_file_hash(absolute_path)?;
        let cached_hash = cached_map.get(path).map(|cached| cached.hash.as_str());
        if cached_hash != Some(hash.as_str()) {
            changed.insert(path.clone());
        }
    }

    Ok(changed)
}

fn compute_file_hash(path: &Path) -> io::Result<String> {
    let bytes = fs::read(path)?;
    Ok(fnv1a_hash(&bytes))
}

fn fnv1a_hash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn summarize_file(absolute_path: &Path, relative_path: &Path) -> FileSummary {
    let location_summary = summarize_location(relative_path);
    let language = detect_language(&location_summary.extension).map(str::to_owned);
    let kind = detect_kind(relative_path, &location_summary).to_owned();
    let content_summary =
        analyze_file_content(absolute_path, location_summary.extension.as_deref());
    let tags = generate_tags(
        relative_path,
        &location_summary,
        content_summary.as_ref(),
        language.as_deref(),
        &kind,
    );

    let path = normalize_path(relative_path);

    FileSummary {
        file_id: path.clone(),
        path,
        language,
        kind,
        location_summary,
        content_summary,
        tags,
    }
}

fn summarize_location(relative_path: &Path) -> LocationSummary {
    let parts = path_parts(relative_path);
    let basename = parts
        .last()
        .cloned()
        .unwrap_or_else(|| normalize_path(relative_path));
    let dirs = parts[..parts.len().saturating_sub(1)].to_vec();
    let stem = Path::new(&basename)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(&basename)
        .to_owned();
    let extension = Path::new(&basename)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);

    let mut path_tokens = Vec::new();
    for dir in &dirs {
        path_tokens.extend(tokenize_segment(dir));
    }
    path_tokens.extend(tokenize_segment(&stem));
    if let Some(ext) = &extension {
        path_tokens.push(ext.clone());
    }

    let mut path_patterns = Vec::new();
    for depth in 1..=dirs.len() {
        path_patterns.push(format!("{}/*", dirs[..depth].join("/")));
    }

    LocationSummary {
        depth: parts.len(),
        dirs,
        basename,
        stem,
        extension,
        path_tokens,
        path_patterns,
    }
}

fn generate_tags(
    relative_path: &Path,
    location: &LocationSummary,
    content: Option<&ContentSummary>,
    language: Option<&str>,
    kind: &str,
) -> Vec<Tag> {
    let mut tags = Vec::new();
    let mut seen = HashSet::new();

    push_tag(
        &mut tags,
        &mut seen,
        format!("depth:{}", location.depth),
        "path",
        1.0,
        vec![format!("path={}", normalize_path(relative_path))],
    );

    for dir in &location.dirs {
        push_tag(
            &mut tags,
            &mut seen,
            format!("dir:{dir}"),
            "path",
            1.0,
            vec![format!("dir={dir}")],
        );
    }

    push_tag(
        &mut tags,
        &mut seen,
        format!("name:{}", location.stem),
        "filename",
        1.0,
        vec![format!("stem={}", location.stem)],
    );

    push_tag(
        &mut tags,
        &mut seen,
        format!("basename:{}", location.basename),
        "filename",
        1.0,
        vec![format!("basename={}", location.basename)],
    );

    if let Some(ext) = &location.extension {
        push_tag(
            &mut tags,
            &mut seen,
            format!("ext:{ext}"),
            "path",
            1.0,
            vec![format!("extension={ext}")],
        );
    }

    for pattern in &location.path_patterns {
        push_tag(
            &mut tags,
            &mut seen,
            format!("path-pattern:{pattern}"),
            "path",
            1.0,
            vec![format!("matched={pattern}")],
        );
    }

    if !location.dirs.is_empty() {
        push_tag(
            &mut tags,
            &mut seen,
            format!("path:{}", location.dirs.join("/")),
            "path",
            1.0,
            vec![format!("dirs={}", location.dirs.join("/"))],
        );
    }

    push_tag(
        &mut tags,
        &mut seen,
        format!("kind:{kind}"),
        "path",
        1.0,
        vec![format!("kind={kind}")],
    );

    if let Some(language) = language {
        push_tag(
            &mut tags,
            &mut seen,
            format!("lang:{language}"),
            "path",
            1.0,
            vec![format!("language={language}")],
        );
    }

    if let Some(content) = content {
        for import in &content.imports {
            push_tag(
                &mut tags,
                &mut seen,
                format!("import:{import}"),
                "content",
                0.85,
                vec![format!("import={import}")],
            );
        }

        for export in &content.exports {
            push_tag(
                &mut tags,
                &mut seen,
                format!("export:{export}"),
                "content",
                0.85,
                vec![format!("export={export}")],
            );
        }

        for symbol in &content.symbols {
            push_tag(
                &mut tags,
                &mut seen,
                format!("symbol:{symbol}"),
                "content",
                0.8,
                vec![format!("symbol={symbol}")],
            );
        }

        for keyword in &content.keywords {
            push_tag(
                &mut tags,
                &mut seen,
                format!("kw:{keyword}"),
                "content",
                0.7,
                vec![format!("keyword={keyword}")],
            );
        }

        for tech in &content.tech {
            push_tag(
                &mut tags,
                &mut seen,
                format!("tech:{tech}"),
                "content",
                0.85,
                vec![format!("tech={tech}")],
            );
        }

        for role in &content.roles {
            push_tag(
                &mut tags,
                &mut seen,
                format!("role:{role}"),
                "content",
                0.8,
                vec![format!("content-role={role}")],
            );
        }

        for side_effect in &content.side_effects {
            push_tag(
                &mut tags,
                &mut seen,
                format!("side-effect:{side_effect}"),
                "content",
                0.85,
                vec![format!("side-effect={side_effect}")],
            );
        }
    }

    for role in derive_roles(relative_path, location) {
        push_tag(
            &mut tags,
            &mut seen,
            format!("role:{}", role.value),
            role.source,
            role.confidence,
            role.evidence,
        );
    }

    tags.sort_by(|left, right| left.value.cmp(&right.value));
    tags
}

fn analyze_file_content(absolute_path: &Path, extension: Option<&str>) -> Option<ContentSummary> {
    let content = fs::read_to_string(absolute_path).ok()?;
    Some(analyze_content(&content, extension))
}

fn analyze_content(content: &str, extension: Option<&str>) -> ContentSummary {
    let rust_ast = if extension == Some("rs") {
        analyze_rust_file(content)
    } else {
        None
    };
    let other_ast = analyze_other_file(content, extension);

    let imports = merge_values(
        merge_values(
            rust_ast
                .as_ref()
                .map(|summary| summary.imports.clone())
                .unwrap_or_default(),
            other_ast
                .as_ref()
                .map(|summary| summary.imports.clone())
                .unwrap_or_default(),
        ),
        extract_imports(content, extension),
    );
    let exports = merge_values(
        merge_values(
            rust_ast
                .as_ref()
                .map(|summary| summary.exports.clone())
                .unwrap_or_default(),
            other_ast
                .as_ref()
                .map(|summary| summary.exports.clone())
                .unwrap_or_default(),
        ),
        extract_exports(content, extension),
    );
    let symbols = merge_values(
        merge_values(
            rust_ast
                .as_ref()
                .map(|summary| summary.symbols.clone())
                .unwrap_or_default(),
            other_ast
                .as_ref()
                .map(|summary| summary.symbols.clone())
                .unwrap_or_default(),
        ),
        extract_symbols(content, extension),
    );
    let keywords = extract_keywords(content, &imports, &exports, &symbols);
    let tech = detect_tech(content, extension, &imports);
    let roles = merge_values(
        merge_values(
            rust_ast
                .as_ref()
                .map(|summary| summary.roles.clone())
                .unwrap_or_default(),
            other_ast
                .as_ref()
                .map(|summary| summary.roles.clone())
                .unwrap_or_default(),
        ),
        derive_content_roles(content),
    );
    let side_effects = detect_side_effects(content);
    let metrics = compute_metrics(content);

    ContentSummary {
        imports,
        exports,
        symbols,
        keywords,
        tech,
        roles,
        side_effects,
        metrics,
    }
}

fn compute_metrics(content: &str) -> ContentMetrics {
    let mut line_count = 0;
    let mut non_empty_line_count = 0;
    let mut comment_line_count = 0;

    for line in content.lines() {
        line_count += 1;
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            non_empty_line_count += 1;
        }
        if is_comment_line(trimmed) {
            comment_line_count += 1;
        }
    }

    ContentMetrics {
        byte_count: content.len(),
        line_count,
        non_empty_line_count,
        comment_line_count,
    }
}

fn is_comment_line(trimmed: &str) -> bool {
    trimmed.starts_with("//")
        || trimmed.starts_with('#')
        || trimmed.starts_with("/*")
        || trimmed.starts_with('*')
        || trimmed.starts_with("--")
}

fn extract_imports(content: &str, extension: Option<&str>) -> Vec<String> {
    let mut imports = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();

        match extension {
            Some("rs") => {
                if let Some(rest) = trimmed
                    .strip_prefix("use ")
                    .or_else(|| trimmed.strip_prefix("pub use "))
                    && let Some(import_target) = parse_rust_use_target(rest)
                {
                    imports.push(import_target);
                }
            }
            Some("ts") | Some("tsx") | Some("js") | Some("jsx") => {
                if trimmed.starts_with("import ") {
                    if let Some(module_name) = extract_quoted_value_after(trimmed, " from ") {
                        imports.push(module_name);
                    } else if let Some(module_name) = extract_quoted_value(trimmed) {
                        imports.push(module_name);
                    }
                }
                if let Some(require_target) = extract_require_target(trimmed) {
                    imports.push(require_target);
                }
            }
            Some("py") => {
                if let Some(rest) = trimmed.strip_prefix("import ") {
                    for segment in rest.split(',') {
                        if let Some(import_target) = normalize_token(segment) {
                            imports.push(import_target);
                        }
                    }
                } else if let Some(rest) = trimmed.strip_prefix("from ") {
                    let target = rest.split_whitespace().next().unwrap_or_default();
                    if let Some(import_target) = normalize_token(target) {
                        imports.push(import_target);
                    }
                }
            }
            _ => {}
        }
    }

    unique_values(imports)
}

fn extract_exports(content: &str, extension: Option<&str>) -> Vec<String> {
    let mut exports = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();

        match extension {
            Some("rs") => {
                for prefix in [
                    "pub fn ",
                    "pub struct ",
                    "pub enum ",
                    "pub trait ",
                    "pub mod ",
                ] {
                    if let Some(symbol) = parse_identifier_after_prefix(trimmed, prefix) {
                        exports.push(symbol);
                    }
                }
            }
            Some("ts") | Some("tsx") | Some("js") | Some("jsx") => {
                for prefix in [
                    "export function ",
                    "export class ",
                    "export const ",
                    "export let ",
                    "export interface ",
                    "export type ",
                ] {
                    if let Some(symbol) = parse_identifier_after_prefix(trimmed, prefix) {
                        exports.push(symbol);
                    }
                }
            }
            _ => {}
        }
    }

    unique_values(exports)
}

fn extract_symbols(content: &str, extension: Option<&str>) -> Vec<String> {
    let mut symbols = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();

        match extension {
            Some("rs") => {
                for prefix in [
                    "fn ",
                    "pub fn ",
                    "struct ",
                    "pub struct ",
                    "enum ",
                    "pub enum ",
                    "trait ",
                    "pub trait ",
                    "mod ",
                    "pub mod ",
                ] {
                    if let Some(symbol) = parse_identifier_after_prefix(trimmed, prefix) {
                        symbols.push(symbol);
                    }
                }
            }
            Some("ts") | Some("tsx") | Some("js") | Some("jsx") => {
                for prefix in [
                    "function ",
                    "export function ",
                    "class ",
                    "export class ",
                    "const ",
                    "export const ",
                    "interface ",
                    "export interface ",
                    "type ",
                    "export type ",
                ] {
                    if let Some(symbol) = parse_identifier_after_prefix(trimmed, prefix) {
                        symbols.push(symbol);
                    }
                }
            }
            Some("py") => {
                for prefix in ["def ", "class "] {
                    if let Some(symbol) = parse_identifier_after_prefix(trimmed, prefix) {
                        symbols.push(symbol);
                    }
                }
            }
            _ => {}
        }
    }

    unique_values(symbols)
}

fn extract_keywords(
    content: &str,
    imports: &[String],
    exports: &[String],
    symbols: &[String],
) -> Vec<String> {
    let mut frequencies = HashMap::new();
    let mut excluded = HashSet::new();

    for value in imports.iter().chain(exports).chain(symbols) {
        excluded.insert(value.to_owned());
    }

    for token in tokenize_content(content) {
        if token.len() < 3 || is_stopword(&token) || excluded.contains(&token) {
            continue;
        }
        *frequencies.entry(token).or_insert(0usize) += 1;
    }

    let mut keywords = frequencies.into_iter().collect::<Vec<_>>();
    keywords.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));

    keywords
        .into_iter()
        .take(8)
        .map(|(keyword, _)| keyword)
        .collect()
}

fn detect_tech(content: &str, extension: Option<&str>, imports: &[String]) -> Vec<String> {
    let lower = content.to_ascii_lowercase();
    let mut tech = Vec::new();

    for (name, needles) in [
        ("serde", &["serde"][..]),
        ("tokio", &["tokio"][..]),
        ("reqwest", &["reqwest"][..]),
        ("sqlx", &["sqlx"][..]),
        ("diesel", &["diesel"][..]),
        ("axum", &["axum"][..]),
        ("actix", &["actix"][..]),
        ("react", &["react"][..]),
        ("nextjs", &["next", "next/"][..]),
        ("openai", &["openai"][..]),
        ("tailwind", &["tailwind"][..]),
        ("prisma", &["prisma"][..]),
        ("sqlite", &["sqlite"][..]),
        ("postgres", &["postgres", "postgresql"][..]),
        ("mysql", &["mysql"][..]),
        (
            "http",
            &["http://", "https://", "http", "fetch(", "axios"][..],
        ),
    ] {
        if needles.iter().any(|needle| lower.contains(needle))
            || imports.iter().any(|item| item.contains(name))
        {
            tech.push(name.to_owned());
        }
    }

    if extension == Some("rs") {
        tech.push("rust".to_owned());
    }

    unique_values(tech)
}

fn derive_content_roles(content: &str) -> Vec<String> {
    let lower = content.to_ascii_lowercase();
    let mut roles = Vec::new();

    if lower.contains("#[test]") || lower.contains("mod tests") || lower.contains("describe(") {
        roles.push("test".to_owned());
    }

    unique_values(roles)
}

fn detect_side_effects(content: &str) -> Vec<String> {
    let lower = content.to_ascii_lowercase();
    let mut side_effects = Vec::new();

    if [
        "http://",
        "https://",
        "reqwest",
        "fetch(",
        "axios",
        "hyper",
        "tcpstream",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        side_effects.push("network".to_owned());
    }
    if [
        "std::fs",
        "tokio::fs",
        "read_to_string",
        "write(",
        "file::open",
        "open(",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        side_effects.push("file-io".to_owned());
    }
    if [
        "sql", "sqlx", "diesel", "prisma", "select ", "insert ", "update ",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        side_effects.push("database".to_owned());
    }
    if ["println!", "eprintln!", "console.log", "print("]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        side_effects.push("stdout".to_owned());
    }
    if ["std::env", "process.env", "dotenv", "env::var"]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        side_effects.push("env".to_owned());
    }

    unique_values(side_effects)
}

fn parse_rust_use_target(rest: &str) -> Option<String> {
    let trimmed = rest.trim().trim_end_matches(';');
    let token = trimmed
        .split(" as ")
        .next()
        .unwrap_or(trimmed)
        .split(',')
        .next()
        .unwrap_or(trimmed)
        .split("::{")
        .next()
        .unwrap_or(trimmed);
    normalize_token(token)
}

fn extract_quoted_value_after(line: &str, marker: &str) -> Option<String> {
    let (_, rest) = line.split_once(marker)?;
    extract_quoted_value(rest)
}

fn extract_quoted_value(line: &str) -> Option<String> {
    let start = line.find(['"', '\''])?;
    let quote = line[start..].chars().next()?;
    let rest = &line[start + quote.len_utf8()..];
    let end = rest.find(quote)?;
    normalize_token(&rest[..end])
}

fn extract_require_target(line: &str) -> Option<String> {
    let start = line.find("require(")?;
    extract_quoted_value(&line[start + "require(".len()..])
}

fn parse_identifier_after_prefix(line: &str, prefix: &str) -> Option<String> {
    let rest = line.strip_prefix(prefix)?;
    let identifier = rest
        .chars()
        .take_while(|char| char.is_ascii_alphanumeric() || *char == '_')
        .collect::<String>();
    normalize_token(&identifier)
}

fn normalize_token(value: &str) -> Option<String> {
    let normalized = value
        .trim()
        .trim_matches(|char: char| matches!(char, '"' | '\'' | ';' | ',' | '{' | '}' | '(' | ')'))
        .to_ascii_lowercase();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn tokenize_content(content: &str) -> Vec<String> {
    content
        .split(|char: char| !char.is_ascii_alphanumeric() && char != '_')
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn unique_values(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();

    for value in values {
        if seen.insert(value.clone()) {
            unique.push(value);
        }
    }

    unique
}

fn merge_values(primary: Vec<String>, secondary: Vec<String>) -> Vec<String> {
    let mut merged = primary;
    merged.extend(secondary);
    unique_values(merged)
}

fn is_stopword(token: &str) -> bool {
    matches!(
        token,
        "the"
            | "and"
            | "for"
            | "with"
            | "from"
            | "into"
            | "this"
            | "that"
            | "true"
            | "false"
            | "none"
            | "some"
            | "self"
            | "pub"
            | "use"
            | "mod"
            | "let"
            | "const"
            | "impl"
            | "enum"
            | "struct"
            | "trait"
            | "type"
            | "class"
            | "function"
            | "return"
            | "async"
            | "await"
            | "else"
            | "match"
            | "while"
            | "loop"
            | "break"
            | "continue"
            | "crate"
            | "super"
            | "string"
            | "result"
            | "error"
            | "value"
            | "args"
            | "path"
            | "line"
            | "lines"
            | "file"
            | "files"
            | "json"
    )
}

fn push_tag(
    tags: &mut Vec<Tag>,
    seen: &mut HashSet<String>,
    value: String,
    source: &str,
    confidence: f32,
    evidence: Vec<String>,
) {
    if seen.insert(value.clone()) {
        tags.push(Tag {
            value,
            source: source.to_owned(),
            confidence,
            evidence,
        });
    }
}

struct DerivedRole {
    value: &'static str,
    source: &'static str,
    confidence: f32,
    evidence: Vec<String>,
}

fn derive_roles(relative_path: &Path, location: &LocationSummary) -> Vec<DerivedRole> {
    let mut roles = Vec::new();
    let basename = location.basename.as_str();
    let dirs = &location.dirs;

    if is_entrypoint_file(basename) || dirs.iter().any(|dir| matches!(dir.as_str(), "bin" | "cmd"))
    {
        roles.push(DerivedRole {
            value: "entrypoint",
            source: "convention",
            confidence: 0.95,
            evidence: vec![format!("basename={basename}")],
        });
    }

    if basename == "lib.rs" {
        roles.push(DerivedRole {
            value: "library-root",
            source: "convention",
            confidence: 0.95,
            evidence: vec![format!("basename={basename}")],
        });
    }

    if basename == "mod.rs" {
        roles.push(DerivedRole {
            value: "module-root",
            source: "convention",
            confidence: 0.95,
            evidence: vec![format!("basename={basename}")],
        });
    }

    if basename.eq_ignore_ascii_case("README.md")
        || dirs.iter().any(|dir| dir == "docs")
        || location.extension.as_deref() == Some("md")
    {
        roles.push(DerivedRole {
            value: "docs",
            source: "convention",
            confidence: 0.9,
            evidence: vec![format!("path={}", normalize_path(relative_path))],
        });
    }

    if is_test_file(relative_path, location) {
        roles.push(DerivedRole {
            value: "test",
            source: "convention",
            confidence: 0.95,
            evidence: vec![format!("basename={basename}")],
        });
    }

    if is_manifest_file(basename) {
        roles.push(DerivedRole {
            value: "manifest",
            source: "filename",
            confidence: 1.0,
            evidence: vec![format!("basename={basename}")],
        });
    }

    if basename == "Dockerfile" {
        roles.push(DerivedRole {
            value: "container",
            source: "filename",
            confidence: 1.0,
            evidence: vec![format!("basename={basename}")],
        });
    }

    roles
}

fn detect_language(extension: &Option<String>) -> Option<&'static str> {
    match extension.as_deref() {
        Some("rs") => Some("rust"),
        Some("ts") | Some("tsx") => Some("typescript"),
        Some("js") | Some("jsx") => Some("javascript"),
        Some("py") => Some("python"),
        Some("go") => Some("go"),
        Some("java") => Some("java"),
        Some("kt") => Some("kotlin"),
        Some("md") => Some("markdown"),
        Some("toml") => Some("toml"),
        Some("json") => Some("json"),
        Some("yaml") | Some("yml") => Some("yaml"),
        _ => None,
    }
}

fn detect_kind(relative_path: &Path, location: &LocationSummary) -> &'static str {
    let basename = location.basename.as_str();

    if is_vendor_path(relative_path) {
        return "vendor";
    }

    if is_artifact_path(relative_path) {
        return "artifact";
    }

    if is_generated_file(relative_path, location) {
        return "generated";
    }

    if is_manifest_file(basename) {
        return "manifest";
    }

    if is_lockfile(basename) {
        return "lockfile";
    }

    if location.extension.as_deref() == Some("md") || basename.eq_ignore_ascii_case("README.md") {
        return "docs";
    }

    if location.extension.as_deref() == Some("toml")
        || matches!(location.extension.as_deref(), Some("yaml" | "yml" | "json"))
    {
        return "config";
    }

    if is_test_file(relative_path, location) {
        return "test";
    }

    if detect_language(&location.extension).is_some() {
        return "source";
    }

    "unknown"
}

fn is_test_file(relative_path: &Path, location: &LocationSummary) -> bool {
    let basename = location.basename.as_str();
    location.dirs.iter().any(|dir| dir == "tests")
        || basename.contains(".test.")
        || basename.contains(".spec.")
        || location.stem.ends_with("_test")
        || normalize_path(relative_path).contains("/tests/")
}

fn is_entrypoint_file(basename: &str) -> bool {
    matches!(
        basename,
        "main.rs"
            | "main.py"
            | "main.go"
            | "main.ts"
            | "main.tsx"
            | "main.js"
            | "main.jsx"
            | "__main__.py"
    )
}

fn is_manifest_file(basename: &str) -> bool {
    matches!(
        basename,
        "Cargo.toml" | "package.json" | "pyproject.toml" | "go.mod"
    )
}

fn is_lockfile(basename: &str) -> bool {
    matches!(
        basename,
        "Cargo.lock" | "package-lock.json" | "pnpm-lock.yaml" | "yarn.lock"
    )
}

fn is_vendor_path(path: &Path) -> bool {
    path_contains_any_component(path, &["node_modules", "vendor"])
}

fn is_artifact_path(path: &Path) -> bool {
    path_contains_any_component(
        path,
        &[
            "target",
            "dist",
            "build",
            "coverage",
            ".next",
            ".nuxt",
            ".svelte-kit",
            ".turbo",
            "out",
        ],
    )
}

fn is_generated_file(relative_path: &Path, location: &LocationSummary) -> bool {
    let basename = location.basename.as_str().to_ascii_lowercase();
    let stem = location.stem.as_str().to_ascii_lowercase();

    path_contains_any_component(relative_path, &["generated", "__generated__", "gen"])
        || basename.contains(".generated.")
        || basename.ends_with(".generated")
        || basename.ends_with(".gen.rs")
        || basename.ends_with(".gen.ts")
        || basename.ends_with(".gen.js")
        || stem.ends_with("_generated")
}

fn path_parts(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => part.to_str().map(str::to_owned),
            _ => None,
        })
        .collect()
}

fn path_contains_component(path: &Path, needle: &str) -> bool {
    path.components().any(|component| match component {
        Component::Normal(part) => part == needle,
        _ => false,
    })
}

fn path_contains_any_component(path: &Path, needles: &[&str]) -> bool {
    needles
        .iter()
        .any(|needle| path_contains_component(path, needle))
}

fn tokenize_segment(segment: &str) -> Vec<String> {
    segment
        .split(|char: char| !char.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn normalize_path(path: &Path) -> String {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        if let Component::Normal(part) = component {
            normalized.push(part);
        }
    }
    normalized.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        ScanMode, analyze_content, detect_kind, files_index_path, generate_tags, load_tags_index,
        scan_project, scan_project_with_mode, summarize_location, tags_index_path,
    };

    fn temp_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("proj-finder-scanner-{name}-{nanos}"))
    }

    #[test]
    fn location_summary_contains_expected_path_fields() {
        let path = Path::new("src/features/auth/service.rs");
        let summary = summarize_location(path);

        assert_eq!(summary.depth, 4);
        assert_eq!(summary.dirs, vec!["src", "features", "auth"]);
        assert_eq!(summary.basename, "service.rs");
        assert_eq!(summary.stem, "service");
        assert_eq!(summary.extension.as_deref(), Some("rs"));
        assert_eq!(
            summary.path_patterns,
            vec!["src/*", "src/features/*", "src/features/auth/*"]
        );
    }

    #[test]
    fn generated_tags_include_path_and_role_information() {
        let path = Path::new("src/main.rs");
        let summary = summarize_location(path);
        let content = analyze_content(
            "use serde::Serialize;\nfn main() { println!(\"hi\"); }\n",
            Some("rs"),
        );
        let tags = generate_tags(path, &summary, Some(&content), Some("rust"), "source");
        let values = tags
            .iter()
            .map(|tag| tag.value.as_str())
            .collect::<Vec<_>>();

        assert!(values.contains(&"depth:2"));
        assert!(values.contains(&"dir:src"));
        assert!(values.contains(&"ext:rs"));
        assert!(values.contains(&"lang:rust"));
        assert!(values.contains(&"import:serde::serialize"));
        assert!(values.contains(&"side-effect:stdout"));
        assert!(values.contains(&"role:entrypoint"));
    }

    #[test]
    fn docs_files_receive_docs_role() {
        let path = Path::new("docs/api/auth.md");
        let summary = summarize_location(path);
        let content = analyze_content(
            "# Auth API\nThis document describes login authentication tokens.\n",
            Some("md"),
        );
        let tags = generate_tags(path, &summary, Some(&content), Some("markdown"), "docs");
        let values = tags
            .iter()
            .map(|tag| tag.value.as_str())
            .collect::<Vec<_>>();

        assert!(values.contains(&"role:docs"));
        assert!(values.contains(&"kind:docs"));
        assert!(values.contains(&"kw:auth"));
    }

    #[test]
    fn content_summary_extracts_imports_symbols_and_tech() {
        let content = r#"
use reqwest::Client;
use std::fs;

pub fn fetch_user() {
    println!("hello");
}
"#;

        let summary = analyze_content(content, Some("rs"));

        assert!(summary.imports.contains(&"reqwest::client".to_owned()));
        assert!(summary.imports.contains(&"std::fs".to_owned()));
        assert!(summary.symbols.contains(&"fetch_user".to_owned()));
        assert!(summary.exports.contains(&"fetch_user".to_owned()));
        assert!(summary.tech.contains(&"reqwest".to_owned()));
        assert!(summary.side_effects.contains(&"file-io".to_owned()));
        assert!(summary.side_effects.contains(&"stdout".to_owned()));
    }

    #[test]
    fn content_summary_ignores_entrypoint_strings_in_source_text() {
        let content = r##"
fn parse_fixture() -> &'static str {
    r#"fn main() {}"#
}
"##;

        let summary = analyze_content(content, Some("rs"));

        assert!(!summary.roles.contains(&"entrypoint".to_owned()));
    }

    #[test]
    fn non_rust_main_files_receive_entrypoint_role() {
        for (path, language) in [
            ("src/main.py", "python"),
            ("cmd/main.go", "go"),
            ("src/main.ts", "typescript"),
            ("src/main.js", "javascript"),
        ] {
            let path = Path::new(path);
            let summary = summarize_location(path);
            let tags = generate_tags(path, &summary, None, Some(language), "source");
            let values = tags
                .iter()
                .map(|tag| tag.value.as_str())
                .collect::<Vec<_>>();

            assert!(values.contains(&"role:entrypoint"));
        }
    }

    #[test]
    fn generated_vendor_and_artifact_paths_are_classified_by_kind() {
        for (path, expected_kind, language) in [
            (
                "src/__generated__/client.ts",
                "generated",
                Some("typescript"),
            ),
            ("target/debug/proj-finder", "artifact", None),
            ("node_modules/react/index.js", "vendor", Some("javascript")),
        ] {
            let path = Path::new(path);
            let summary = summarize_location(path);
            let kind = detect_kind(path, &summary);
            let tags = generate_tags(path, &summary, None, language, kind);
            let values = tags
                .iter()
                .map(|tag| tag.value.as_str())
                .collect::<Vec<_>>();

            assert_eq!(kind, expected_kind);
            assert!(values.contains(&format!("kind:{expected_kind}").as_str()));
        }
    }

    #[test]
    fn scan_project_respects_gitignore() {
        let root = temp_path("gitignore");
        fs::create_dir_all(root.join(".git")).expect("should create fake git dir");
        fs::create_dir_all(root.join("src")).expect("should create src dir");
        fs::write(root.join(".gitignore"), "ignored.txt\nignored-dir/\n")
            .expect("should write gitignore");
        fs::write(root.join("kept.txt"), "keep").expect("should write kept file");
        fs::write(root.join("ignored.txt"), "ignore").expect("should write ignored file");
        fs::create_dir_all(root.join("ignored-dir")).expect("should create ignored dir");
        fs::write(root.join("ignored-dir").join("nested.txt"), "ignore")
            .expect("should write nested ignored file");

        let result = scan_project(&root).expect("scan should succeed");
        let scanned = result
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>();

        assert!(scanned.contains(&".gitignore"));
        assert!(scanned.contains(&"kept.txt"));
        assert!(!scanned.contains(&"ignored.txt"));
        assert!(!scanned.contains(&"ignored-dir/nested.txt"));

        fs::remove_dir_all(root).expect("should clean up temp dir");
    }

    #[test]
    fn scan_project_excludes_git_directory_contents() {
        let root = temp_path("exclude-dot-git");
        fs::create_dir_all(root.join(".git")).expect("should create fake git dir");
        fs::create_dir_all(root.join("src")).expect("should create src dir");
        fs::write(root.join(".git").join("HEAD"), "ref: refs/heads/main\n")
            .expect("should write git head");
        fs::write(root.join("src").join("main.rs"), "fn main() {}\n")
            .expect("should write source file");

        let result = scan_project(&root).expect("scan should succeed");
        let scanned = result
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>();

        assert!(scanned.contains(&"src/main.rs"));
        assert!(!scanned.contains(&".git/HEAD"));

        fs::remove_dir_all(root).expect("should clean up temp dir");
    }

    #[test]
    fn incremental_scan_persists_index_and_updates_changed_files() {
        let root = temp_path("incremental-update");
        fs::create_dir_all(root.join(".git")).expect("should create fake git dir");
        fs::create_dir_all(root.join("src")).expect("should create src dir");
        fs::write(root.join("src").join("main.rs"), "fn main() {}\n")
            .expect("should write source file");

        let first = scan_project_with_mode(&root, ScanMode::Incremental)
            .expect("first incremental scan should succeed");
        assert!(files_index_path(&root).exists());
        assert!(tags_index_path(&root).exists());
        assert!(
            !first
                .files
                .iter()
                .any(|file| file.path.starts_with(".proj-finder/"))
        );

        fs::write(
            root.join("src").join("main.rs"),
            "use reqwest::Client;\nfn main() {}\n",
        )
        .expect("should update source file");

        let second = scan_project_with_mode(&root, ScanMode::Incremental)
            .expect("second incremental scan should succeed");
        let main_file = second
            .files
            .iter()
            .find(|file| file.path == "src/main.rs")
            .expect("main file should exist");
        let imports = &main_file
            .content_summary
            .as_ref()
            .expect("main file should have content summary")
            .imports;

        assert!(imports.contains(&"reqwest::client".to_owned()));

        fs::remove_dir_all(root).expect("should clean up temp dir");
    }

    #[test]
    fn persisted_tags_index_deduplicates_values_and_builds_postings() {
        let root = temp_path("tag-index");
        fs::create_dir_all(root.join(".git")).expect("should create fake git dir");
        fs::create_dir_all(root.join("src")).expect("should create src dir");
        fs::write(root.join("src").join("main.rs"), "fn main() {}\n")
            .expect("should write main file");
        fs::write(root.join("src").join("lib.rs"), "pub fn run() {}\n")
            .expect("should write library file");

        scan_project_with_mode(&root, ScanMode::Incremental)
            .expect("incremental scan should succeed");
        let tags_index = load_tags_index(&root)
            .expect("tag index should load")
            .expect("tag index should exist");

        assert_eq!(
            tags_index
                .strings
                .iter()
                .filter(|value| value.as_str() == "lang:rust")
                .count(),
            1
        );

        let lang_rust_id = tags_index
            .strings
            .iter()
            .position(|value| value == "lang:rust")
            .expect("lang:rust should be interned") as u32;
        let posting = tags_index
            .inverted_index
            .iter()
            .find(|posting| posting.value_id == lang_rust_id)
            .expect("lang:rust should have a posting list");
        assert_eq!(posting.file_indices.len(), 2);

        fs::remove_dir_all(root).expect("should clean up temp dir");
    }

    #[test]
    fn incremental_scan_removes_deleted_files() {
        let root = temp_path("incremental-remove");
        fs::create_dir_all(root.join(".git")).expect("should create fake git dir");
        fs::create_dir_all(root.join("src")).expect("should create src dir");
        fs::write(root.join("src").join("main.rs"), "fn main() {}\n")
            .expect("should write main file");
        fs::write(root.join("src").join("lib.rs"), "pub fn run() {}\n")
            .expect("should write library file");

        scan_project_with_mode(&root, ScanMode::Incremental)
            .expect("first incremental scan should succeed");
        fs::remove_file(root.join("src").join("lib.rs")).expect("should remove library file");

        let second = scan_project_with_mode(&root, ScanMode::Incremental)
            .expect("second incremental scan should succeed");

        assert!(second.files.iter().any(|file| file.path == "src/main.rs"));
        assert!(!second.files.iter().any(|file| file.path == "src/lib.rs"));

        fs::remove_dir_all(root).expect("should clean up temp dir");
    }

    #[test]
    fn incremental_scan_falls_back_when_index_is_invalid() {
        let root = temp_path("incremental-invalid-index");
        fs::create_dir_all(root.join(".git")).expect("should create fake git dir");
        fs::create_dir_all(root.join("src")).expect("should create src dir");
        fs::create_dir_all(root.join(".proj-finder")).expect("should create index dir");
        fs::write(root.join(".proj-finder").join("files.bin"), "not-bincode")
            .expect("should write invalid index");
        fs::write(root.join("src").join("main.rs"), "fn main() {}\n")
            .expect("should write source file");

        let result = scan_project_with_mode(&root, ScanMode::Incremental)
            .expect("incremental scan should fall back to full scan");

        assert!(result.files.iter().any(|file| file.path == "src/main.rs"));

        fs::remove_dir_all(root).expect("should clean up temp dir");
    }
}
