//! Selector parsing, canonicalization, storage keys, and coverage algebra.

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::resolve_lexical;
use crate::identity::{DEFAULT_SOURCE_ID, SourceId};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectorKind {
    All,
    DateRange,
    Cwd,
    CwdDateRange,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Selector {
    All {
        source: SourceId,
        root: String,
    },
    DateRange {
        source: SourceId,
        root: String,
        from_date: String,
        to_date: String,
    },
    Cwd {
        source: SourceId,
        root: String,
        cwd: String,
    },
    CwdDateRange {
        source: SourceId,
        root: String,
        cwd: String,
        from_date: String,
        to_date: String,
    },
}

impl Selector {
    pub const fn kind(&self) -> SelectorKind {
        match self {
            Self::All { .. } => SelectorKind::All,
            Self::DateRange { .. } => SelectorKind::DateRange,
            Self::Cwd { .. } => SelectorKind::Cwd,
            Self::CwdDateRange { .. } => SelectorKind::CwdDateRange,
        }
    }

    pub const fn source(&self) -> SourceId {
        match self {
            Self::All { source, .. }
            | Self::DateRange { source, .. }
            | Self::Cwd { source, .. }
            | Self::CwdDateRange { source, .. } => *source,
        }
    }

    pub fn root(&self) -> &str {
        match self {
            Self::All { root, .. }
            | Self::DateRange { root, .. }
            | Self::Cwd { root, .. }
            | Self::CwdDateRange { root, .. } => root,
        }
    }

    pub fn storage_key(&self) -> String {
        let kind = match self.kind() {
            SelectorKind::All => "all",
            SelectorKind::DateRange => "date_range",
            SelectorKind::Cwd => "cwd",
            SelectorKind::CwdDateRange => "cwd_date_range",
        };
        let source = quote(self.source().as_str());
        let root = quote(self.root());

        match self {
            Self::All { .. } => {
                format!(r#"{{"kind":"{kind}","source":{source},"root":{root}}}"#)
            }
            Self::DateRange {
                from_date, to_date, ..
            } => format!(
                r#"{{"kind":"{kind}","source":{source},"root":{root},"fromDate":{},"toDate":{}}}"#,
                quote(from_date),
                quote(to_date)
            ),
            Self::Cwd { cwd, .. } => format!(
                r#"{{"kind":"{kind}","source":{source},"root":{root},"cwd":{}}}"#,
                quote(cwd)
            ),
            Self::CwdDateRange {
                cwd,
                from_date,
                to_date,
                ..
            } => format!(
                r#"{{"kind":"{kind}","source":{source},"root":{root},"cwd":{},"fromDate":{},"toDate":{}}}"#,
                quote(cwd),
                quote(from_date),
                quote(to_date)
            ),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum RawSelector {
    All {
        #[serde(default)]
        source: Option<SourceId>,
        #[serde(default)]
        root: Option<String>,
    },
    DateRange {
        #[serde(default)]
        source: Option<SourceId>,
        #[serde(default)]
        root: Option<String>,
        #[serde(default)]
        from_date: Option<String>,
        #[serde(default)]
        to_date: Option<String>,
    },
    Cwd {
        #[serde(default)]
        source: Option<SourceId>,
        #[serde(default)]
        root: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
    },
    CwdDateRange {
        #[serde(default)]
        source: Option<SourceId>,
        #[serde(default)]
        root: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        from_date: Option<String>,
        #[serde(default)]
        to_date: Option<String>,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SelectorDefaults {
    pub default_root: Option<String>,
    pub default_source: Option<SourceId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SelectorError {
    InvalidJson(String),
    InvalidShape(String),
    MissingOrEmpty(&'static str),
    InvalidDate(&'static str),
    ReversedDateRange,
    NonUtf8Root,
}

impl fmt::Display for SelectorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(error) => write!(formatter, "invalid selector JSON: {error}"),
            Self::InvalidShape(error) => write!(formatter, "invalid selector: {error}"),
            Self::MissingOrEmpty(field) => {
                write!(formatter, "selector.{field} must be a non-empty string")
            }
            Self::InvalidDate(field) => write!(formatter, "selector.{field} must be YYYY-MM-DD"),
            Self::ReversedDateRange => formatter.write_str("fromDate must be <= toDate"),
            Self::NonUtf8Root => formatter.write_str("selector.root must be valid UTF-8"),
        }
    }
}

impl std::error::Error for SelectorError {}

pub fn parse_selector_json(
    value: &str,
    defaults: &SelectorDefaults,
    cwd: &Path,
) -> Result<Selector, SelectorError> {
    let value: serde_json::Value = serde_json::from_str(value)
        .map_err(|error| SelectorError::InvalidJson(error.to_string()))?;
    let raw = serde_json::from_value(value)
        .map_err(|error| SelectorError::InvalidShape(error.to_string()))?;
    canonicalize_selector(raw, defaults, cwd)
}

pub fn canonicalize_selector(
    raw: RawSelector,
    defaults: &SelectorDefaults,
    cwd: &Path,
) -> Result<Selector, SelectorError> {
    match raw {
        RawSelector::All { source, root } => Ok(Selector::All {
            source: source
                .or(defaults.default_source)
                .unwrap_or(DEFAULT_SOURCE_ID),
            root: canonical_root(root, defaults, cwd)?,
        }),
        RawSelector::DateRange {
            source,
            root,
            from_date,
            to_date,
        } => {
            let (from_date, to_date) = canonical_dates(from_date, to_date)?;
            Ok(Selector::DateRange {
                source: source
                    .or(defaults.default_source)
                    .unwrap_or(DEFAULT_SOURCE_ID),
                root: canonical_root(root, defaults, cwd)?,
                from_date,
                to_date,
            })
        }
        RawSelector::Cwd {
            source,
            root,
            cwd: selected_cwd,
        } => Ok(Selector::Cwd {
            source: source
                .or(defaults.default_source)
                .unwrap_or(DEFAULT_SOURCE_ID),
            root: canonical_root(root, defaults, cwd)?,
            cwd: required_string(selected_cwd, "cwd")?,
        }),
        RawSelector::CwdDateRange {
            source,
            root,
            cwd: selected_cwd,
            from_date,
            to_date,
        } => {
            let (from_date, to_date) = canonical_dates(from_date, to_date)?;
            Ok(Selector::CwdDateRange {
                source: source
                    .or(defaults.default_source)
                    .unwrap_or(DEFAULT_SOURCE_ID),
                root: canonical_root(root, defaults, cwd)?,
                cwd: required_string(selected_cwd, "cwd")?,
                from_date,
                to_date,
            })
        }
    }
}

pub fn selector_implies(covering: &Selector, requested: &Selector) -> bool {
    if covering.source() != requested.source() || covering.root() != requested.root() {
        return false;
    }

    match covering {
        Selector::All { .. } => true,
        Selector::DateRange {
            from_date, to_date, ..
        } => match requested {
            Selector::DateRange {
                from_date: requested_from,
                to_date: requested_to,
                ..
            }
            | Selector::CwdDateRange {
                from_date: requested_from,
                to_date: requested_to,
                ..
            } => contains_date_range(from_date, to_date, requested_from, requested_to),
            _ => false,
        },
        Selector::Cwd {
            cwd: covering_cwd, ..
        } => match requested {
            Selector::Cwd {
                cwd: requested_cwd, ..
            }
            | Selector::CwdDateRange {
                cwd: requested_cwd, ..
            } => covering_cwd == requested_cwd,
            _ => false,
        },
        Selector::CwdDateRange {
            cwd: covering_cwd,
            from_date,
            to_date,
            ..
        } => match requested {
            Selector::CwdDateRange {
                cwd: requested_cwd,
                from_date: requested_from,
                to_date: requested_to,
                ..
            } => {
                covering_cwd == requested_cwd
                    && contains_date_range(from_date, to_date, requested_from, requested_to)
            }
            _ => false,
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelectorFile<'a> {
    pub path_date: Option<&'a str>,
    pub cwd: &'a str,
}

pub fn selector_contains_file(selector: &Selector, file: SelectorFile<'_>) -> bool {
    match selector {
        Selector::All { .. } => true,
        Selector::Cwd { cwd, .. } => file.cwd == cwd,
        Selector::DateRange {
            from_date, to_date, ..
        } => file
            .path_date
            .is_some_and(|date| date_in_range(date, from_date, to_date)),
        Selector::CwdDateRange {
            cwd,
            from_date,
            to_date,
            ..
        } => {
            file.cwd == cwd
                && file
                    .path_date
                    .is_some_and(|date| date_in_range(date, from_date, to_date))
        }
    }
}

pub fn selector_storage_key(selector: &Selector) -> String {
    selector.storage_key()
}

fn canonical_root(
    root: Option<String>,
    defaults: &SelectorDefaults,
    cwd: &Path,
) -> Result<String, SelectorError> {
    let root = root.or_else(|| defaults.default_root.clone());
    let root = required_string(root, "root")?;
    resolve_lexical(root, cwd)
        .into_os_string()
        .into_string()
        .map_err(|_| SelectorError::NonUtf8Root)
}

fn canonical_dates(
    from_date: Option<String>,
    to_date: Option<String>,
) -> Result<(String, String), SelectorError> {
    let from_date = required_date(from_date, "fromDate")?;
    let to_date = required_date(to_date, "toDate")?;
    if from_date > to_date {
        return Err(SelectorError::ReversedDateRange);
    }
    Ok((from_date, to_date))
}

fn required_string(value: Option<String>, field: &'static str) -> Result<String, SelectorError> {
    let value = value.ok_or(SelectorError::MissingOrEmpty(field))?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(SelectorError::MissingOrEmpty(field));
    }
    Ok(trimmed.to_owned())
}

fn required_date(value: Option<String>, field: &'static str) -> Result<String, SelectorError> {
    let value = required_string(value, field)?;
    let bytes = value.as_bytes();
    let has_shape = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit());
    if !has_shape {
        return Err(SelectorError::InvalidDate(field));
    }
    Ok(value)
}

fn contains_date_range(
    covering_from: &str,
    covering_to: &str,
    requested_from: &str,
    requested_to: &str,
) -> bool {
    covering_from <= requested_from && covering_to >= requested_to
}

fn date_in_range(date: &str, from_date: &str, to_date: &str) -> bool {
    date >= from_date && date <= to_date
}

fn quote(value: &str) -> String {
    serde_json::to_string(value).expect("serializing a string cannot fail")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "/sessions";

    fn all(source: SourceId, root: &str) -> Selector {
        Selector::All {
            source,
            root: root.to_owned(),
        }
    }

    fn date(source: SourceId, root: &str, from_date: &str, to_date: &str) -> Selector {
        Selector::DateRange {
            source,
            root: root.to_owned(),
            from_date: from_date.to_owned(),
            to_date: to_date.to_owned(),
        }
    }

    fn cwd(source: SourceId, root: &str, value: &str) -> Selector {
        Selector::Cwd {
            source,
            root: root.to_owned(),
            cwd: value.to_owned(),
        }
    }

    fn cwd_date(
        source: SourceId,
        root: &str,
        value: &str,
        from_date: &str,
        to_date: &str,
    ) -> Selector {
        Selector::CwdDateRange {
            source,
            root: root.to_owned(),
            cwd: value.to_owned(),
            from_date: from_date.to_owned(),
            to_date: to_date.to_owned(),
        }
    }

    #[test]
    fn parses_defaults_trims_values_and_resolves_root_lexically() {
        let selector = parse_selector_json(
            r#"{"kind":"cwd","cwd":"  /repo/项目  "}"#,
            &SelectorDefaults {
                default_root: Some("../raw/./sessions".to_owned()),
                default_source: Some(SourceId::Pi),
            },
            Path::new("/work/project"),
        )
        .unwrap();
        assert_eq!(
            selector,
            cwd(SourceId::Pi, "/work/raw/sessions", "/repo/项目")
        );
    }

    #[test]
    fn storage_keys_match_node_canonical_property_order() {
        let cases = [
            (
                all(SourceId::Codex, ROOT),
                r#"{"kind":"all","source":"codex","root":"/sessions"}"#,
            ),
            (
                date(SourceId::ClaudeCode, ROOT, "2026-01-01", "2026-01-31"),
                r#"{"kind":"date_range","source":"claude-code","root":"/sessions","fromDate":"2026-01-01","toDate":"2026-01-31"}"#,
            ),
            (
                cwd(SourceId::Pi, ROOT, "/work/\"quoted\""),
                r#"{"kind":"cwd","source":"pi","root":"/sessions","cwd":"/work/\"quoted\""}"#,
            ),
            (
                cwd_date(SourceId::Codex, ROOT, "/work", "2026-02-01", "2026-02-28"),
                r#"{"kind":"cwd_date_range","source":"codex","root":"/sessions","cwd":"/work","fromDate":"2026-02-01","toDate":"2026-02-28"}"#,
            ),
        ];

        for (selector, expected) in cases {
            assert_eq!(selector_storage_key(&selector), expected);
            assert_eq!(serde_json::to_string(&selector).unwrap(), expected);
        }
    }

    #[test]
    fn implication_matrix_matches_coverage_algebra() {
        let all_codex = all(SourceId::Codex, ROOT);
        let month = date(SourceId::Codex, ROOT, "2026-01-01", "2026-01-31");
        let week = date(SourceId::Codex, ROOT, "2026-01-08", "2026-01-14");
        let repo = cwd(SourceId::Codex, ROOT, "/repo");
        let repo_week = cwd_date(SourceId::Codex, ROOT, "/repo", "2026-01-08", "2026-01-14");
        let other_repo_week = cwd_date(SourceId::Codex, ROOT, "/other", "2026-01-08", "2026-01-14");

        for requested in [&all_codex, &month, &week, &repo, &repo_week] {
            assert!(selector_implies(&all_codex, requested));
        }
        assert!(selector_implies(&month, &week));
        assert!(selector_implies(&month, &repo_week));
        assert!(selector_implies(&repo, &repo_week));
        assert!(!selector_implies(&week, &month));
        assert!(!selector_implies(&month, &repo));
        assert!(!selector_implies(&repo, &other_repo_week));
        assert!(!selector_implies(&all_codex, &all(SourceId::Pi, ROOT)));
        assert!(!selector_implies(
            &all_codex,
            &all(SourceId::Codex, "/other-root")
        ));
    }

    #[test]
    fn file_membership_requires_dates_only_for_date_selectors() {
        let missing_date = SelectorFile {
            path_date: None,
            cwd: "/repo",
        };
        assert!(selector_contains_file(
            &all(SourceId::Codex, ROOT),
            missing_date
        ));
        assert!(selector_contains_file(
            &cwd(SourceId::Codex, ROOT, "/repo"),
            missing_date
        ));
        assert!(!selector_contains_file(
            &date(SourceId::Codex, ROOT, "2026-01-01", "2026-01-31"),
            missing_date
        ));

        let dated = SelectorFile {
            path_date: Some("2026-01-31"),
            cwd: "/repo",
        };
        assert!(selector_contains_file(
            &date(SourceId::Codex, ROOT, "2026-01-01", "2026-01-31"),
            dated
        ));
        assert!(selector_contains_file(
            &cwd_date(SourceId::Codex, ROOT, "/repo", "2026-01-01", "2026-01-31"),
            dated
        ));
    }

    #[test]
    fn rejects_invalid_shapes_and_dates() {
        let defaults = SelectorDefaults {
            default_root: Some(ROOT.to_owned()),
            default_source: None,
        };
        assert!(matches!(
            parse_selector_json("not-json", &defaults, Path::new("/work")),
            Err(SelectorError::InvalidJson(_))
        ));
        assert!(matches!(
            parse_selector_json("[]", &defaults, Path::new("/work")),
            Err(SelectorError::InvalidShape(_))
        ));
        let invalid = [
            "not-json",
            r#"[]"#,
            r#"{"kind":"unknown"}"#,
            r#"{"kind":"cwd","cwd":"  "}"#,
            r#"{"kind":"date_range","fromDate":"2026-1-01","toDate":"2026-01-02"}"#,
            r#"{"kind":"date_range","fromDate":"2026-02-01","toDate":"2026-01-01"}"#,
            r#"{"kind":"all","source":"future"}"#,
        ];
        for value in invalid {
            assert!(
                parse_selector_json(value, &defaults, Path::new("/work")).is_err(),
                "selector should be rejected: {value}"
            );
        }
    }

    #[test]
    fn missing_root_is_rejected_without_a_default() {
        assert_eq!(
            parse_selector_json(
                r#"{"kind":"all"}"#,
                &SelectorDefaults::default(),
                Path::new("/work")
            ),
            Err(SelectorError::MissingOrEmpty("root"))
        );
    }
}
