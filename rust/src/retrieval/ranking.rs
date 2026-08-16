use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::identity::SourceId;
use crate::model::{FindMatchRole, FindResult, FindSort, MatchSource};
use crate::tokenizer::tokenize;

use super::candidate::{
    CandidateEvidence, content_text, match_role, match_source, matched_fields, session_group_key,
};
use super::query::{QueryAnalysis, RecallMode, analyze_query};
use super::snippet::{make_like_snippet, make_raw_snippet};
use super::utf16;

const RELEVANCE_RECALL_MULTIPLIER: usize = 12;
const MIN_RELEVANCE_CANDIDATES: usize = 50;
const TIME_RECALL_MULTIPLIER: usize = 4;
const MIN_TIME_CANDIDATES: usize = 50;
const TIME_RERANK_SESSION_MULTIPLIER: usize = 2;

/// Upstream stores use this to bound candidate reads before invoking policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetrievalPlan {
    pub candidate_limit: usize,
    pub full_rerank: bool,
}

impl RetrievalPlan {
    pub fn for_sort(limit: usize, sort: FindSort) -> Self {
        match sort {
            FindSort::Relevance => Self {
                candidate_limit: limit
                    .saturating_mul(RELEVANCE_RECALL_MULTIPLIER)
                    .max(MIN_RELEVANCE_CANDIDATES),
                full_rerank: true,
            },
            FindSort::Ended | FindSort::Started => Self {
                // The SQL/document query is already ordered by time. The old
                // TypeScript path fetched at least 1,000 rows and fully
                // tokenized/reranked all of them before restoring time order.
                candidate_limit: limit
                    .saturating_mul(TIME_RECALL_MULTIPLIER)
                    .max(MIN_TIME_CANDIDATES),
                full_rerank: false,
            },
        }
    }
}

struct SessionAggregate {
    row: CandidateEvidence,
    best_row: CandidateEvidence,
    best_display_row: CandidateEvidence,
    best_row_signal_score: f64,
    best_display_row_signal_score: f64,
    hit_count: u64,
    session_hit_count: u64,
    user_hit_count: u64,
    title_phrase: bool,
    title_term_hits: usize,
    cwd_term_hits: usize,
    title_restatement: bool,
    insertion_order: usize,
}

pub fn rank_candidates(
    candidates: &[CandidateEvidence],
    query: &str,
    limit: usize,
) -> Vec<FindResult> {
    rank_candidates_at(candidates, query, limit, unix_time_millis())
}

pub fn rank_candidates_at(
    candidates: &[CandidateEvidence],
    query: &str,
    limit: usize,
    now_millis: i64,
) -> Vec<FindResult> {
    let analysis = analyze_query(query);
    let mut grouped: HashMap<String, SessionAggregate> = HashMap::new();
    let mut key_order = Vec::new();

    for candidate in candidates {
        let signal_score = score_row(candidate, &analysis);
        let key = session_group_key(candidate);
        if let Some(existing) = grouped.get_mut(&key) {
            update_aggregate(existing, candidate, signal_score);
        } else {
            let insertion_order = key_order.len();
            key_order.push(key.clone());
            grouped.insert(
                key,
                create_aggregate(candidate, &analysis, signal_score, insertion_order),
            );
        }
    }

    let mut ranked = key_order
        .into_iter()
        .filter_map(|key| grouped.remove(&key))
        .map(|aggregate| {
            let score = score_session(&aggregate, now_millis);
            (aggregate, score)
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|(left, left_score), (right, right_score)| {
        right_score
            .total_cmp(left_score)
            .then_with(|| right.row.ended_at.cmp(&left.row.ended_at))
            .then_with(|| left.insertion_order.cmp(&right.insertion_order))
    });

    ranked
        .into_iter()
        .take(limit)
        .enumerate()
        .map(|(index, (aggregate, score))| to_find_result(aggregate, score, index, &analysis))
        .collect()
}

pub fn rank_candidates_for_sort(
    candidates: &[CandidateEvidence],
    query: &str,
    sort: FindSort,
    limit: usize,
) -> Vec<FindResult> {
    rank_candidates_for_sort_at(candidates, query, sort, limit, unix_time_millis())
}

pub fn rank_candidates_for_sort_at(
    candidates: &[CandidateEvidence],
    query: &str,
    sort: FindSort,
    limit: usize,
    now_millis: i64,
) -> Vec<FindResult> {
    if sort == FindSort::Relevance {
        return rank_candidates_at(candidates, query, limit, now_millis);
    }
    if limit == 0 {
        return vec![];
    }

    // Cheap O(N) grouping and ISO timestamp comparison first. Only a small
    // number of newest sessions pass through tokenization and heuristic score.
    let mut newest_by_session: HashMap<String, (&str, usize)> = HashMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        newest_by_session
            .entry(session_group_key(candidate))
            .or_insert_with(|| {
                let timestamp = match sort {
                    FindSort::Started => candidate.started_at.as_str(),
                    FindSort::Ended => candidate.ended_at.as_str(),
                    FindSort::Relevance => unreachable!(),
                };
                (timestamp, index)
            });
    }
    let mut sessions = newest_by_session.into_iter().collect::<Vec<_>>();
    sessions.sort_by(
        |(left_key, (left_time, left_index)), (right_key, (right_time, right_index))| {
            right_time
                .cmp(left_time)
                .then_with(|| left_index.cmp(right_index))
                .then_with(|| left_key.cmp(right_key))
        },
    );
    let rerank_session_limit = limit
        .saturating_mul(TIME_RERANK_SESSION_MULTIPLIER)
        .max(limit)
        .min(sessions.len());
    let selected = sessions
        .into_iter()
        .take(rerank_session_limit)
        .map(|(key, _)| key)
        .collect::<HashSet<_>>();
    let narrowed = candidates
        .iter()
        .filter(|candidate| selected.contains(&session_group_key(candidate)))
        .cloned()
        .collect::<Vec<_>>();
    let mut results = rank_candidates_at(&narrowed, query, rerank_session_limit, now_millis);
    results.sort_by(|left, right| compare_by_time(left, right, sort));
    results.truncate(limit);
    for (index, result) in results.iter_mut().enumerate() {
        result.rank = (index + 1) as u64;
    }
    results
}

fn create_aggregate(
    candidate: &CandidateEvidence,
    analysis: &QueryAnalysis,
    signal_score: f64,
    insertion_order: usize,
) -> SessionAggregate {
    let title_lower = candidate.title.to_lowercase();
    let cwd_lower = candidate.cwd.to_lowercase();
    let title_phrase = !analysis.normalized_query.is_empty()
        && if analysis.is_path_like_command {
            contains_bounded_phrase(&title_lower, &analysis.normalized_query)
        } else {
            title_lower.contains(&analysis.normalized_query)
        };
    let title_restatement = title_looks_like_command_restatement(&title_lower, analysis);
    let title_term_hits = if title_restatement {
        0
    } else {
        count_matched_terms(&title_lower, &analysis.terms)
    };
    let cwd_term_hits = count_matched_terms(&cwd_lower, &analysis.terms);

    SessionAggregate {
        row: candidate.clone(),
        best_row: candidate.clone(),
        best_display_row: candidate.clone(),
        best_row_signal_score: signal_score,
        best_display_row_signal_score: signal_score,
        hit_count: 1,
        session_hit_count: u64::from(match_source(candidate) == MatchSource::Session),
        user_hit_count: u64::from(match_role(candidate) == FindMatchRole::User),
        title_phrase: title_phrase && !title_restatement,
        title_term_hits,
        cwd_term_hits,
        title_restatement,
        insertion_order,
    }
}

fn update_aggregate(
    aggregate: &mut SessionAggregate,
    candidate: &CandidateEvidence,
    signal_score: f64,
) {
    aggregate.hit_count += 1;
    aggregate.session_hit_count += u64::from(match_source(candidate) == MatchSource::Session);
    aggregate.user_hit_count += u64::from(match_role(candidate) == FindMatchRole::User);
    if signal_score > aggregate.best_row_signal_score {
        aggregate.best_row = candidate.clone();
        aggregate.best_row_signal_score = signal_score;
    }
    if should_use_display_row(
        &aggregate.best_display_row,
        candidate,
        aggregate.best_display_row_signal_score,
        signal_score,
    ) {
        aggregate.best_display_row = candidate.clone();
        aggregate.best_display_row_signal_score = signal_score;
    }
}

fn should_use_display_row(
    current: &CandidateEvidence,
    candidate: &CandidateEvidence,
    current_score: f64,
    candidate_score: f64,
) -> bool {
    if match_source(candidate) == MatchSource::Message
        && match_source(current) != MatchSource::Message
    {
        return true;
    }
    if match_source(candidate) != match_source(current) {
        return false;
    }
    candidate_score > current_score
}

fn score_row(candidate: &CandidateEvidence, analysis: &QueryAnalysis) -> f64 {
    let content_lower = content_text(candidate).to_lowercase();
    let content_phrase = !analysis.normalized_query.is_empty()
        && if analysis.is_path_like_command {
            contains_bounded_phrase(&content_lower, &analysis.normalized_query)
        } else {
            content_lower.contains(&analysis.normalized_query)
        };
    let term_coverage = count_matched_terms(&content_lower, &analysis.terms);
    -candidate.fts_score
        + if content_phrase { 8.0 } else { 0.0 }
        + score_path_like_command_sequence(&content_lower, analysis)
        + term_coverage as f64 * 2.0
        + if match_source(candidate) == MatchSource::Message {
            4.0
        } else {
            0.0
        }
        + if match_role(candidate) == FindMatchRole::User {
            2.0
        } else {
            0.0
        }
}

fn score_session(aggregate: &SessionAggregate, now_millis: i64) -> f64 {
    aggregate.best_row_signal_score
        + if aggregate.title_phrase { 30.0 } else { 0.0 }
        + aggregate.title_term_hits as f64 * 10.0
        + aggregate.cwd_term_hits as f64 * 18.0
        + aggregate.user_hit_count.min(3) as f64 * 4.0
        + aggregate.session_hit_count.min(2) as f64 * 2.0
        + aggregate.hit_count.min(6) as f64 * 1.5
        + recency_decay(&aggregate.row.ended_at, now_millis)
        - if aggregate.title_restatement {
            20.0
        } else {
            0.0
        }
}

fn recency_decay(ended_at: &str, now_millis: i64) -> f64 {
    let Ok(timestamp) = ended_at.parse::<jiff::Timestamp>() else {
        return 0.0;
    };
    let elapsed = (now_millis - timestamp.as_millisecond()).max(0) as f64;
    let days = elapsed / 86_400_000.0;
    (18.0 - days * 0.15).max(0.0)
}

fn to_find_result(
    aggregate: SessionAggregate,
    score: f64,
    index: usize,
    analysis: &QueryAnalysis,
) -> FindResult {
    let display = &aggregate.best_display_row;
    let display_content = content_text(display);
    let snippet = match &analysis.recall {
        RecallMode::Like { needle } => make_like_snippet(&display_content, needle),
        RecallMode::Empty | RecallMode::Fts { .. } => make_raw_snippet(
            &display_content,
            &analysis.normalized_query,
            &analysis.terms,
        ),
    };
    let matched_fields = matched_fields(display, &analysis.normalized_query, &analysis.terms);
    let session_ref = session_ref_for_result(
        display.source_id,
        &display.session_uuid,
        &display.session_key,
    );
    FindResult {
        rank: (index + 1) as u64,
        source_id: display.source_id,
        session_uuid: display.session_uuid.clone(),
        session_ref,
        title: aggregate.row.title,
        summary_text: aggregate.row.summary_text,
        cwd: aggregate.row.cwd,
        started_at: aggregate.row.started_at,
        ended_at: aggregate.row.ended_at,
        match_count: aggregate.hit_count,
        match_source: match_source(display),
        match_seq: display.seq,
        match_role: match_role(display),
        match_timestamp: display.timestamp.clone(),
        score,
        snippet,
        matched_fields,
        session_message_count: aggregate.row.session_message_count,
    }
}

fn session_ref_for_result(source_id: SourceId, session_uuid: &str, session_key: &str) -> String {
    if source_id == SourceId::Codex {
        session_uuid.to_owned()
    } else if session_key.is_empty() {
        format!("{source_id}:{session_uuid}")
    } else {
        session_key.to_owned()
    }
}

fn count_matched_terms(haystack: &str, terms: &[String]) -> usize {
    terms.iter().filter(|term| haystack.contains(*term)).count()
}

fn score_path_like_command_sequence(haystack: &str, analysis: &QueryAnalysis) -> f64 {
    if !analysis.is_path_like_command || analysis.terms.len() < 2 {
        return 0.0;
    }
    if contains_bounded_phrase(haystack, &analysis.normalized_query) {
        return 36.0 + score_trailing_command_args(haystack, &analysis.normalized_query);
    }
    let tokens = tokenize(haystack);
    let Some(span) = shortest_ordered_span(&tokens, &analysis.terms) else {
        return 0.0;
    };
    let gaps = span - analysis.terms.len();
    if gaps == 0 {
        8.0
    } else if gaps <= 3 {
        (24 - gaps * 2) as f64
    } else if gaps <= analysis.terms.len() {
        10.0 - gaps as f64
    } else {
        0.0
    }
}

fn shortest_ordered_span(tokens: &[String], terms: &[String]) -> Option<usize> {
    let mut best = usize::MAX;
    for start in 0..tokens.len() {
        if tokens[start] != terms[0] {
            continue;
        }
        let mut term_index = 1;
        let mut end = start;
        while term_index < terms.len() && end + 1 < tokens.len() {
            end += 1;
            if tokens[end] == terms[term_index] {
                term_index += 1;
            }
        }
        if term_index == terms.len() {
            best = best.min(end - start + 1);
        }
    }
    (best != usize::MAX).then_some(best)
}

fn title_looks_like_command_restatement(title: &str, analysis: &QueryAnalysis) -> bool {
    if !analysis.is_path_like_command
        || analysis.normalized_query.is_empty()
        || !contains_bounded_phrase(title, &analysis.normalized_query)
    {
        return false;
    }
    tokenize(&title.replace(&analysis.normalized_query, " ")).len() >= 2
}

fn score_trailing_command_args(haystack: &str, phrase: &str) -> f64 {
    let mut offset = 0;
    let phrase_len = utf16::len(phrase);
    let haystack_len = utf16::len(haystack);
    while offset < haystack_len {
        let Some(index) = utf16::find_from(haystack, phrase, offset) else {
            return 0.0;
        };
        let after_index = index + phrase_len;
        if is_phrase_boundary(utf16::char_before(haystack, index))
            && is_phrase_boundary(utf16::char_at(haystack, after_index))
        {
            let window = utf16::slice(haystack, after_index, (after_index + 80).min(haystack_len));
            let line = window.split('\n').next().unwrap_or_default();
            if line.contains('.') || line.contains('/') || contains_flag_like_argument(line) {
                return 24.0;
            }
        }
        offset = index + 1;
    }
    0.0
}

fn contains_flag_like_argument(line: &str) -> bool {
    line.split_whitespace().any(|token| {
        let token = token.strip_prefix("--").or_else(|| token.strip_prefix('-'));
        token.is_some_and(|value| {
            value
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_alphanumeric())
        })
    })
}

fn contains_bounded_phrase(haystack: &str, phrase: &str) -> bool {
    if phrase.is_empty() {
        return false;
    }
    let mut offset = 0;
    let phrase_len = utf16::len(phrase);
    while offset < utf16::len(haystack) {
        let Some(index) = utf16::find_from(haystack, phrase, offset) else {
            return false;
        };
        let after = index + phrase_len;
        if is_phrase_boundary(utf16::char_before(haystack, index))
            && is_phrase_boundary(utf16::char_at(haystack, after))
        {
            return true;
        }
        offset = index + 1;
    }
    false
}

fn is_phrase_boundary(character: Option<char>) -> bool {
    character.is_none_or(|character| {
        !(character.is_alphanumeric() || matches!(character, '_' | '.' | '/' | '-'))
    })
}

fn compare_by_time(left: &FindResult, right: &FindResult, sort: FindSort) -> std::cmp::Ordering {
    let (left_time, right_time) = match sort {
        FindSort::Started => (&left.started_at, &right.started_at),
        FindSort::Ended => (&left.ended_at, &right.ended_at),
        FindSort::Relevance => unreachable!(),
    };
    right_time
        .cmp(left_time)
        .then_with(|| right.score.total_cmp(&left.score))
        .then_with(|| left.source_id.cmp(&right.source_id))
        .then_with(|| left.session_ref.cmp(&right.session_ref))
}

fn unix_time_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}
