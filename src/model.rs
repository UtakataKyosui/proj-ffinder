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

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct SearchQuery {
    #[serde(default)]
    pub must: Vec<String>,
    #[serde(default)]
    pub any: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub prefer: Vec<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub root: String,
    pub query: SearchQuery,
    pub hits: Vec<SearchHit>,
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
