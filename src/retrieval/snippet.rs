use std::collections::HashSet;

use super::utf16;

const BEFORE_CONTEXT: usize = 40;
const AFTER_CONTEXT: usize = 80;
const FALLBACK_LENGTH: usize = 160;

pub fn make_like_snippet(content: &str, query: &str) -> String {
    let lower = content.to_lowercase();
    let target = query.to_lowercase();
    let Some(index) = utf16::find_from(&lower, &target, 0) else {
        return utf16::slice(content, 0, FALLBACK_LENGTH);
    };
    let target_len = utf16::len(&target);
    let content_len = utf16::len(content);
    let start = index.saturating_sub(BEFORE_CONTEXT);
    let end = (index + target_len + AFTER_CONTEXT).min(content_len);
    let prefix = if start > 0 { "…" } else { "" };
    let suffix = if end < content_len { "…" } else { "" };
    let snippet = utf16::slice(content, start, end);
    format!(
        "{prefix}{}{suffix}",
        wrap_all_occurrences(&snippet, &target)
    )
}

pub fn make_raw_snippet(content: &str, query: &str, terms: &[String]) -> String {
    let normalized_query = query.to_lowercase();
    let lower = content.to_lowercase();
    if !normalized_query.is_empty()
        && let Some(phrase_index) = utf16::find_from(&lower, &normalized_query, 0)
    {
        return snippet_around(
            content,
            phrase_index,
            utf16::len(query),
            &[normalized_query],
        );
    }

    let term_lowers = unique_non_empty(terms.iter().map(|term| term.to_lowercase()));
    let mut best_start = 0;
    let mut best_end = 0;
    let mut best_anchor = usize::MAX;
    let mut best_score: i64 = -1;
    let mut any_hit = false;
    let content_len = utf16::len(content);

    for term in &term_lowers {
        let term_len = utf16::len(term);
        let mut cursor = 0;
        while cursor < utf16::len(&lower) {
            let Some(index) = utf16::find_from(&lower, term, cursor) else {
                break;
            };
            any_hit = true;
            let start = index.saturating_sub(BEFORE_CONTEXT);
            let end = (index + term_len + AFTER_CONTEXT).min(content_len);
            let score = score_snippet_window(&utf16::slice(&lower, start, end), &term_lowers);
            if score > best_score || (score == best_score && index < best_anchor) {
                best_start = start;
                best_end = end;
                best_anchor = index;
                best_score = score;
            }
            cursor = index + term_len;
        }
    }

    if !any_hit {
        return utf16::slice(content, 0, FALLBACK_LENGTH);
    }
    snippet_window(content, best_start, best_end, &term_lowers)
}

fn snippet_around(content: &str, index: usize, length: usize, needles: &[String]) -> String {
    let start = index.saturating_sub(BEFORE_CONTEXT);
    let end = (index + length + AFTER_CONTEXT).min(utf16::len(content));
    snippet_window(content, start, end, needles)
}

fn snippet_window(content: &str, start: usize, end: usize, needles: &[String]) -> String {
    let prefix = if start > 0 { "…" } else { "" };
    let suffix = if end < utf16::len(content) { "…" } else { "" };
    let snippet = utf16::slice(content, start, end);
    format!(
        "{prefix}{}{suffix}",
        wrap_any_occurrences(&snippet, needles)
    )
}

fn score_snippet_window(lower: &str, terms: &[String]) -> i64 {
    let mut distinct_terms = 0;
    let mut total_hits = 0;
    let mut matched_chars = 0;
    for term in terms {
        let hits = count_term_hits(lower, term);
        if hits > 0 {
            distinct_terms += 1;
        }
        total_hits += hits;
        matched_chars += hits * utf16::len(term);
    }
    (distinct_terms * 1_000 + matched_chars * 10 + total_hits) as i64
}

fn count_term_hits(lower: &str, term: &str) -> usize {
    let mut count = 0;
    let mut cursor = 0;
    let term_len = utf16::len(term);
    while cursor < utf16::len(lower) {
        let Some(index) = utf16::find_from(lower, term, cursor) else {
            break;
        };
        count += 1;
        cursor = index + term_len;
    }
    count
}

fn unique_non_empty(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| !value.is_empty() && seen.insert(value.clone()))
        .collect()
}

fn collect_term_hits(lower: &str, term: &str) -> Vec<(usize, usize)> {
    let mut hits = Vec::new();
    let mut cursor = 0;
    let term_len = utf16::len(term);
    while cursor < utf16::len(lower) {
        let Some(index) = utf16::find_from(lower, term, cursor) else {
            break;
        };
        hits.push((index, term_len));
        cursor = index + term_len;
    }
    hits
}

fn wrap_any_occurrences(haystack: &str, needles: &[String]) -> String {
    let mut needles = unique_non_empty(needles.iter().cloned());
    needles.sort_by_key(|needle| std::cmp::Reverse(utf16::len(needle)));
    if needles.is_empty() {
        return haystack.to_owned();
    }

    let lower = haystack.to_lowercase();
    let mut matches = needles
        .iter()
        .flat_map(|needle| collect_term_hits(&lower, needle))
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1)));

    let mut output = String::new();
    let mut cursor = 0;
    for (index, length) in matches {
        if index < cursor {
            continue;
        }
        output.push_str(&utf16::slice(haystack, cursor, index));
        output.push_str("<mark>");
        output.push_str(&utf16::slice(haystack, index, index + length));
        output.push_str("</mark>");
        cursor = index + length;
    }
    output.push_str(&utf16::slice(haystack, cursor, utf16::len(haystack)));
    output
}

fn wrap_all_occurrences(haystack: &str, needle: &str) -> String {
    if needle.is_empty() {
        return haystack.to_owned();
    }
    let lower = haystack.to_lowercase();
    let mut output = String::new();
    let mut cursor = 0;
    let needle_len = utf16::len(needle);
    while cursor < utf16::len(haystack) {
        let Some(index) = utf16::find_from(&lower, needle, cursor) else {
            output.push_str(&utf16::slice(haystack, cursor, utf16::len(haystack)));
            break;
        };
        output.push_str(&utf16::slice(haystack, cursor, index));
        output.push_str("<mark>");
        output.push_str(&utf16::slice(haystack, index, index + needle_len));
        output.push_str("</mark>");
        cursor = index + needle_len;
    }
    output
}
