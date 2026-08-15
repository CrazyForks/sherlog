//! Deterministic text normalization shared by indexing and query extraction.

use std::collections::HashSet;

use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

/// Split text into overlapping CJK scalar bigrams and lowercase UAX #29
/// words. Single-scalar CJK runs are intentionally dropped as retrieval noise.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cjk = Vec::new();
    let mut non_cjk = String::new();

    for scalar in text.chars() {
        if is_cjk_scalar(scalar) {
            flush_non_cjk(&mut non_cjk, &mut tokens);
            cjk.push(scalar);
        } else {
            flush_cjk(&mut cjk, &mut tokens);
            non_cjk.push(scalar);
        }
    }
    flush_cjk(&mut cjk, &mut tokens);
    flush_non_cjk(&mut non_cjk, &mut tokens);

    tokens
}

pub fn tokenized_text(text: &str) -> String {
    tokenize(text).join(" ")
}

/// Return distinct tokens while preserving first-seen order.
pub fn query_terms(query: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    tokenize(query)
        .into_iter()
        .filter(|token| seen.insert(token.clone()))
        .collect()
}

pub fn has_cjk(text: &str) -> bool {
    text.chars().any(is_cjk_scalar)
}

pub fn is_cjk_token(token: &str) -> bool {
    !token.is_empty() && token.chars().all(is_cjk_scalar)
}

fn is_cjk_scalar(scalar: char) -> bool {
    matches!(
        scalar.script(),
        Script::Han | Script::Hiragana | Script::Katakana | Script::Hangul
    )
}

fn flush_cjk(buffer: &mut Vec<char>, tokens: &mut Vec<String>) {
    if buffer.len() >= 2 {
        tokens.extend(
            buffer
                .windows(2)
                .map(|pair| pair.iter().collect::<String>()),
        );
    }
    buffer.clear();
}

fn flush_non_cjk(buffer: &mut String, tokens: &mut Vec<String>) {
    if buffer.is_empty() {
        return;
    }
    tokens.extend(buffer.unicode_words().map(str::to_lowercase));
    buffer.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn tokenization_golden_covers_mixed_cjk_latin_and_paths() {
        let input = "Deploy 健康检查 to /Users/RS/sherlog-v2/src/query_flow.ts";
        assert_eq!(
            tokenize(input),
            strings(&[
                "deploy",
                "健康",
                "康检",
                "检查",
                "to",
                "users",
                "rs",
                "sherlog",
                "v2",
                "src",
                "query_flow.ts",
            ])
        );
        assert_eq!(
            tokenized_text(input),
            "deploy 健康 康检 检查 to users rs sherlog v2 src query_flow.ts"
        );
    }

    #[test]
    fn supplementary_han_uses_scalar_not_utf16_bigrams() {
        assert_eq!(tokenize("𠮷野家"), strings(&["𠮷野", "野家"]));
        assert_eq!(tokenize("𠮷A野"), strings(&["a"]));
        assert!(has_cjk("prefix𠮷suffix"));
        assert!(is_cjk_token("𠮷野"));
    }

    #[test]
    fn adjacent_cjk_scripts_form_one_scalar_run() {
        assert_eq!(
            tokenize("中文かな한글"),
            strings(&["中文", "文か", "かな", "な한", "한글"])
        );
    }

    #[test]
    fn isolated_cjk_scalars_are_dropped() {
        assert_eq!(tokenize("汉 A 字"), strings(&["a"]));
        assert_eq!(tokenize("𠮷"), Vec::<String>::new());
    }

    #[test]
    fn unicode_word_segmentation_lowercases_and_ignores_punctuation() {
        assert_eq!(
            tokenize("Hello, can't 32.3 STRASSE Straße sing-box"),
            strings(&["hello", "can't", "32.3", "strasse", "straße", "sing", "box"])
        );
    }

    #[test]
    fn query_terms_deduplicate_without_reordering() {
        assert_eq!(
            query_terms("测试测试 Hello hello 测试"),
            strings(&["测试", "试测", "hello"])
        );
    }

    #[test]
    fn cjk_token_predicate_requires_a_nonempty_pure_cjk_token() {
        assert!(!is_cjk_token(""));
        assert!(!is_cjk_token("中文a"));
        assert!(is_cjk_token("カナ"));
        assert!(is_cjk_token("한글"));
    }
}
