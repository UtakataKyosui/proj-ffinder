use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct ScanResult {
    pub root: String,
    pub files: Vec<FileSummary>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileSummary {
    pub file_id: String,
    pub path: String,
    pub language: Option<String>,
    pub kind: String,
    pub location_summary: LocationSummary,
    pub content_summary: Option<ContentSummary>,
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LocationSummary {
    pub depth: usize,
    pub dirs: Vec<String>,
    pub basename: String,
    pub stem: String,
    pub extension: Option<String>,
    pub path_tokens: Vec<String>,
    pub path_patterns: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Tag {
    pub value: String,
    pub source: String,
    pub confidence: f32,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContentSummary {
    pub imports: Vec<String>,
    pub exports: Vec<String>,
    pub symbols: Vec<String>,
    pub keywords: Vec<String>,
    pub tech: Vec<String>,
    pub roles: Vec<String>,
    pub side_effects: Vec<String>,
    pub metrics: ContentMetrics,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContentMetrics {
    pub byte_count: usize,
    pub line_count: usize,
    pub non_empty_line_count: usize,
    pub comment_line_count: usize,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SearchQuery {
    #[serde(default)]
    pub must: Vec<String>,
    #[serde(default)]
    pub any: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub prefer: Vec<String>,
    #[serde(default)]
    pub grep: Option<GrepQuery>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub root: String,
    pub query: SearchQuery,
    pub hits: Vec<SearchHit>,
    pub assistance: SearchAssistance,
}

#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub file_id: String,
    pub path: String,
    pub kind: String,
    pub language: Option<String>,
    pub score: f32,
    pub matched_must: Vec<String>,
    pub matched_must_details: Vec<MatchedTag>,
    pub matched_any: Vec<String>,
    pub matched_any_details: Vec<MatchedTag>,
    pub matched_prefer: Vec<String>,
    pub matched_prefer_details: Vec<MatchedTag>,
    pub grep_matches: Vec<GrepMatch>,
}

#[derive(Debug, Serialize)]
pub struct SearchAssistance {
    pub suggested_tags: Vec<SuggestedTag>,
    pub suggested_commands: Vec<SuggestedCommand>,
}

#[derive(Debug, Serialize)]
pub struct SuggestedTag {
    pub value: String,
    pub hit_count: usize,
    pub sample_paths: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SuggestedCommand {
    pub description: String,
    pub command: String,
}

fn default_limit() -> usize {
    10
}

#[derive(Debug, Serialize)]
pub struct MatchedTag {
    pub value: String,
    pub source: String,
    pub confidence: f32,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GrepQuery {
    pub pattern: String,
    #[serde(default)]
    pub mode: GrepMode,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub context_before: usize,
    #[serde(default)]
    pub context_after: usize,
    #[serde(default = "default_grep_matches_per_file")]
    pub max_matches_per_file: usize,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GrepMode {
    #[default]
    Literal,
    Regex,
}

#[derive(Debug, Serialize)]
pub struct GrepMatch {
    pub line_number: usize,
    pub line: String,
    pub before: Vec<GrepContextLine>,
    pub after: Vec<GrepContextLine>,
}

#[derive(Debug, Serialize)]
pub struct GrepContextLine {
    pub line_number: usize,
    pub line: String,
}

fn default_grep_matches_per_file() -> usize {
    3
}
