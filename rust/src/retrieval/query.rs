use crate::tokenizer::{has_cjk, query_terms};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecallMode {
    Empty,
    Fts { expression: String },
    Like { needle: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryAnalysis {
    pub normalized_query: String,
    pub terms: Vec<String>,
    pub is_multi_term: bool,
    pub is_path_like_command: bool,
    pub recall: RecallMode,
}

pub fn analyze_query(query: &str) -> QueryAnalysis {
    let trimmed = query.trim();
    let normalized_query = trimmed.to_lowercase();
    let terms = query_terms(trimmed);
    let is_multi_term = !trimmed.is_empty() && trimmed.chars().any(char::is_whitespace);
    let has_path_like_token = query
        .chars()
        .any(|character| matches!(character, '\\' | '/' | '.' | '_' | ':' | '-'));
    let recall = if trimmed.is_empty() {
        RecallMode::Empty
    } else if terms.is_empty() {
        if has_cjk(trimmed) {
            RecallMode::Like {
                needle: trimmed.to_owned(),
            }
        } else {
            RecallMode::Empty
        }
    } else {
        RecallMode::Fts {
            expression: build_fts_match(&terms),
        }
    };

    QueryAnalysis {
        normalized_query,
        terms,
        is_multi_term,
        is_path_like_command: is_multi_term && has_path_like_token,
        recall,
    }
}

/// Build an intersection expression for FTS5 from already-tokenized terms.
pub fn build_fts_match(terms: &[String]) -> String {
    terms
        .iter()
        .map(|term| quote_fts_term(term))
        .collect::<Vec<_>>()
        .join(" AND ")
}

pub fn quote_fts_term(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}

pub fn escape_like_pattern(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}
