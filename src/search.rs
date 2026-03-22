use std::collections::HashMap;

use crate::model::{FileSummary, MatchedTag, SearchHit, SearchQuery, SearchResult, Tag};

pub fn search_files(root: &str, files: &[FileSummary], query: SearchQuery) -> SearchResult {
    let mut hits = files
        .iter()
        .filter_map(|file| score_file(file, &query))
        .collect::<Vec<_>>();

    hits.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.path.cmp(&right.path))
    });
    hits.truncate(query.limit);

    SearchResult {
        root: root.to_owned(),
        query,
        hits,
    }
}

fn score_file(file: &FileSummary, query: &SearchQuery) -> Option<SearchHit> {
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

#[cfg(test)]
mod tests {
    use crate::model::{FileSummary, LocationSummary, SearchQuery, Tag};

    use super::search_files;

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
            limit: 10,
        };

        let result = search_files("/tmp/project", &files, query);

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
            limit: 10,
        };

        let result = search_files("/tmp/project", &files, query);

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
            limit: 10,
        };

        let result = search_files("/tmp/project", &files, query);

        assert_eq!(result.hits.len(), 2);
        assert_eq!(result.hits[0].path, "src/auth/route.rs");
        assert!(result.hits[0].score > result.hits[1].score);
        assert_eq!(result.hits[0].matched_prefer_details.len(), 1);
    }
}
