use crate::model::{MessageElision, MessageElisionStrategy, MessageRecord};
use crate::tokenizer::query_terms;

use super::utf16;

pub const DEFAULT_MAX_MESSAGE_CHARS: usize = 800;

#[derive(Clone, Copy, Debug, Default)]
pub struct ElisionOptions<'a> {
    pub max_message_chars: Option<usize>,
    pub anchor_seq: Option<i64>,
    pub query: Option<&'a str>,
}

pub fn elide_messages(
    messages: &[MessageRecord],
    options: ElisionOptions<'_>,
) -> Vec<MessageRecord> {
    let max_message_chars = options
        .max_message_chars
        .unwrap_or(DEFAULT_MAX_MESSAGE_CHARS);
    if max_message_chars == 0 {
        return messages.to_vec();
    }
    messages
        .iter()
        .map(|message| {
            let query = (Some(message.seq) == options.anchor_seq)
                .then_some(options.query)
                .flatten();
            elide_message(message, max_message_chars, query)
        })
        .collect()
}

fn elide_message(
    message: &MessageRecord,
    max_message_chars: usize,
    query: Option<&str>,
) -> MessageRecord {
    let original_char_count = utf16::len(&message.content_text);
    if original_char_count <= max_message_chars {
        return message.clone();
    }

    let anchor = query.and_then(|query| find_query_anchor(&message.content_text, query));
    let (strategy, preserved) = if let Some((index, length)) = anchor {
        (
            MessageElisionStrategy::AroundQuery,
            preserve_around_query(&message.content_text, max_message_chars, index, length),
        )
    } else {
        (
            MessageElisionStrategy::HeadTail,
            preserve_head_tail(&message.content_text, max_message_chars),
        )
    };
    let omitted_char_count = original_char_count.saturating_sub(preserved.visible_char_count);
    let hint = format!(
        "Rerun this read with --max-message-chars {original_char_count} to inspect the full message."
    );
    let mut result = message.clone();
    result.content_text = preserved.text;
    result.elision = Some(MessageElision {
        original_char_count: original_char_count as u64,
        displayed_char_count: utf16::len(&result.content_text) as u64,
        omitted_char_count: omitted_char_count as u64,
        strategy,
        query: if strategy == MessageElisionStrategy::AroundQuery {
            query.map(str::to_owned)
        } else {
            None
        },
        hint,
    });
    result
}

fn find_query_anchor(text: &str, query: &str) -> Option<(usize, usize)> {
    let text_lower = text.to_lowercase();
    let query_lower = query.to_lowercase();
    if let Some(index) = utf16::find_from(&text_lower, &query_lower, 0) {
        return Some((index, utf16::len(query)));
    }
    let mut terms = query_terms(query);
    terms.sort_by_key(|term| std::cmp::Reverse(utf16::len(term)));
    for term in terms {
        let term_lower = term.to_lowercase();
        if let Some(index) = utf16::find_from(&text_lower, &term_lower, 0) {
            return Some((index, utf16::len(&term)));
        }
    }
    None
}

struct PreservedText {
    text: String,
    visible_char_count: usize,
}

fn preserve_around_query(
    text: &str,
    max_message_chars: usize,
    query_index: usize,
    query_length: usize,
) -> PreservedText {
    let budget = max_message_chars.max(query_length);
    let (start, end) = window_around(utf16::len(text), query_index, query_length, budget);
    if start == 0 {
        return mark_elision(text, start, end, None);
    }
    let head_budget = (budget / 4).min(start);
    if head_budget < 32 {
        return mark_elision(text, start, end, None);
    }
    if start <= head_budget {
        return mark_elision(text, 0, end, None);
    }
    mark_elision(text, 0, head_budget, Some((start, end)))
}

fn window_around(
    text_length: usize,
    query_index: usize,
    query_length: usize,
    budget: usize,
) -> (usize, usize) {
    let start = query_index.saturating_sub((budget.saturating_sub(query_length)) / 2);
    let end = (start + budget).min(text_length);
    (end.saturating_sub(budget), end)
}

fn preserve_head_tail(text: &str, max_message_chars: usize) -> PreservedText {
    let text_length = utf16::len(text);
    let head_count = max_message_chars.div_ceil(2).min(text_length);
    let tail_count = (max_message_chars / 2).min(text_length.saturating_sub(head_count));
    mark_elision(
        text,
        0,
        head_count,
        Some((text_length - tail_count, text_length)),
    )
}

fn mark_elision(
    text: &str,
    first_start: usize,
    first_end: usize,
    second: Option<(usize, usize)>,
) -> PreservedText {
    let text_length = utf16::len(text);
    let (visible, visible_char_count) = if let Some((second_start, second_end)) = second {
        (
            format!(
                "{}\n[... shlog elided middle ...]\n{}",
                utf16::slice(text, first_start, first_end),
                utf16::slice(text, second_start, second_end)
            ),
            (first_end - first_start) + (second_end - second_start),
        )
    } else {
        (
            utf16::slice(text, first_start, first_end),
            first_end - first_start,
        )
    };
    let prefix = if first_start > 0 {
        "[... shlog elided prefix ...]\n"
    } else {
        ""
    };
    let suffix = if second.is_none() && first_end < text_length {
        "\n[... shlog elided suffix ...]"
    } else {
        ""
    };
    PreservedText {
        text: format!("{prefix}{visible}{suffix}"),
        visible_char_count,
    }
}
