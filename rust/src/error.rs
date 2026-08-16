use std::fmt;

use serde_json::{Value, json};

pub const EXIT_SUCCESS: u8 = 0;
pub const EXIT_FAILURE: u8 = 1;
pub const EXIT_INVALID_ARGUMENTS: u8 = 2;

#[derive(Debug)]
pub struct AnchorNotFound {
    pub session_ref: String,
    pub source_id: String,
    pub native_session_id: String,
    pub db_path: String,
    pub query: String,
    pub matched_profile_fields: Vec<String>,
    pub read_page_argv: Vec<String>,
}

#[derive(Debug)]
pub enum AppError {
    UnsupportedOperation {
        operation: &'static str,
    },
    UnsupportedSource {
        source: String,
    },
    ColdRoot {
        message: String,
    },
    InvalidSelector {
        code: &'static str,
        message: String,
    },
    IndexUnavailable {
        db_path: String,
        cwd: String,
        default_root: String,
    },
    IndexSchemaUpgradeRequired {
        message: String,
        db_path: String,
        missing_columns: Vec<String>,
    },
    SessionNotFound {
        session_ref: String,
        source_id: String,
        native_session_id: String,
        db_path: String,
        retry_argv: Vec<String>,
    },
    AnchorNotFound(Box<AnchorNotFound>),
    IndexFailure {
        message: String,
    },
    CommandFailedSilent,
    InvalidArguments {
        message: String,
    },
    Output {
        message: String,
    },
}

impl AppError {
    pub fn unsupported(operation: &'static str) -> Self {
        Self::UnsupportedOperation { operation }
    }

    pub fn invalid_arguments(message: impl Into<String>) -> Self {
        Self::InvalidArguments {
            message: message.into(),
        }
    }

    pub fn unsupported_source(source: impl Into<String>) -> Self {
        let source = source.into();
        let source = if source.trim().is_empty() {
            "(empty)".to_owned()
        } else {
            source.trim().to_owned()
        };
        Self::UnsupportedSource { source }
    }

    pub fn cold_root(message: impl Into<String>) -> Self {
        Self::ColdRoot {
            message: message.into(),
        }
    }

    pub fn invalid_selector(message: impl Into<String>) -> Self {
        let message = message.into();
        let code = if message.contains("requires --selector") {
            "selector_required"
        } else {
            "invalid_selector"
        };
        Self::InvalidSelector { code, message }
    }

    pub fn index_unavailable(
        db_path: impl Into<String>,
        cwd: impl Into<String>,
        default_root: impl Into<String>,
    ) -> Self {
        Self::IndexUnavailable {
            db_path: db_path.into(),
            cwd: cwd.into(),
            default_root: default_root.into(),
        }
    }

    pub fn schema_upgrade_required(
        message: impl Into<String>,
        db_path: impl Into<String>,
        missing_columns: Vec<String>,
    ) -> Self {
        Self::IndexSchemaUpgradeRequired {
            message: message.into(),
            db_path: db_path.into(),
            missing_columns,
        }
    }

    pub fn session_not_found(
        session_ref: impl Into<String>,
        source_id: impl Into<String>,
        native_session_id: impl Into<String>,
        db_path: impl Into<String>,
        retry_argv: Vec<String>,
    ) -> Self {
        Self::SessionNotFound {
            session_ref: session_ref.into(),
            source_id: source_id.into(),
            native_session_id: native_session_id.into(),
            db_path: db_path.into(),
            retry_argv,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn anchor_not_found(
        session_ref: impl Into<String>,
        source_id: impl Into<String>,
        native_session_id: impl Into<String>,
        db_path: impl Into<String>,
        query: impl Into<String>,
        matched_profile_fields: Vec<String>,
        read_page_argv: Vec<String>,
    ) -> Self {
        Self::AnchorNotFound(Box::new(AnchorNotFound {
            session_ref: session_ref.into(),
            source_id: source_id.into(),
            native_session_id: native_session_id.into(),
            db_path: db_path.into(),
            query: query.into(),
            matched_profile_fields,
            read_page_argv,
        }))
    }

    pub fn index_failure(message: impl Into<String>) -> Self {
        Self::IndexFailure {
            message: message.into(),
        }
    }

    pub const fn command_failed_silent() -> Self {
        Self::CommandFailedSilent
    }

    pub fn output(error: impl fmt::Display) -> Self {
        Self::Output {
            message: error.to_string(),
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedOperation { .. } => "unsupported_operation",
            Self::UnsupportedSource { .. } => "unsupported_source",
            Self::ColdRoot { .. } => "invalid_cold_root",
            Self::InvalidSelector { code, .. } => code,
            Self::IndexUnavailable { .. } => "index_unavailable",
            Self::IndexSchemaUpgradeRequired { .. } => "index_schema_upgrade_required",
            Self::SessionNotFound { .. } => "session_not_found",
            Self::AnchorNotFound(_) => "anchor_not_found",
            Self::IndexFailure { .. } => "index_error",
            Self::CommandFailedSilent => "command_failed",
            Self::InvalidArguments { .. } => "invalid_arguments",
            Self::Output { .. } => "output_error",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::UnsupportedOperation { operation } => {
                format!("Rust CLI scaffold does not implement `{operation}` yet.")
            }
            Self::UnsupportedSource { source } => format!(
                "unsupported source \"{source}\". Public sources in this release: codex|claude-code|pi."
            ),
            Self::IndexUnavailable { db_path, .. } => format!("index not found: {db_path}"),
            Self::SessionNotFound { session_ref, .. } => {
                format!("session not found in Sherlog index: {session_ref}")
            }
            Self::AnchorNotFound(payload) => {
                if payload.matched_profile_fields.is_empty() {
                    format!(
                        "query \"{}\" matched no message in session {}",
                        payload.query, payload.session_ref
                    )
                } else {
                    format!(
                        "query \"{}\" matched only session-level fields ({}) in session {}; there is no message anchor",
                        payload.query,
                        payload.matched_profile_fields.join(", "),
                        payload.session_ref
                    )
                }
            }
            Self::CommandFailedSilent => String::new(),
            Self::ColdRoot { message }
            | Self::InvalidSelector { message, .. }
            | Self::IndexSchemaUpgradeRequired { message, .. }
            | Self::IndexFailure { message }
            | Self::InvalidArguments { message }
            | Self::Output { message } => message.clone(),
        }
    }

    pub fn operation(&self) -> Option<&'static str> {
        match self {
            Self::UnsupportedOperation { operation } => Some(operation),
            _ => None,
        }
    }

    pub fn source(&self) -> Option<&str> {
        match self {
            Self::UnsupportedSource { source } => Some(source),
            _ => None,
        }
    }

    pub fn json_uses_stdout(&self) -> bool {
        matches!(
            self,
            Self::UnsupportedSource { .. }
                | Self::ColdRoot { .. }
                | Self::InvalidSelector { .. }
                | Self::IndexUnavailable { .. }
                | Self::IndexSchemaUpgradeRequired { .. }
                | Self::SessionNotFound { .. }
                | Self::AnchorNotFound(_)
                | Self::IndexFailure { .. }
        )
    }

    pub fn plain_message_is_unadorned(&self) -> bool {
        self.json_uses_stdout()
    }

    pub const fn is_silent(&self) -> bool {
        matches!(self, Self::CommandFailedSilent)
    }

    pub fn exit_code(&self) -> u8 {
        match self {
            Self::InvalidArguments { .. } => EXIT_INVALID_ARGUMENTS,
            _ => EXIT_FAILURE,
        }
    }

    pub fn envelope(&self) -> Value {
        let error = match self {
            Self::UnsupportedOperation { operation } => json!({
                "code": self.code(), "message": self.message(), "operation": operation,
            }),
            Self::UnsupportedSource { source } => json!({
                "code": self.code(), "source": source, "message": self.message(),
            }),
            Self::IndexUnavailable {
                db_path,
                cwd,
                default_root,
            } => self.index_unavailable_envelope(db_path, cwd, default_root),
            Self::IndexSchemaUpgradeRequired {
                message,
                db_path,
                missing_columns,
            } => json!({
                "code": self.code(),
                "message": message,
                "dbPath": db_path,
                "missingColumns": missing_columns,
                "hint": "A supported v7 index is migrated only by an explicit scoped `shlog sync`. For a newer or incompatible v8 version/epoch, use a compatible `shlog` binary or restore a trusted backup; repeated sync will not make it compatible.",
            }),
            Self::SessionNotFound {
                session_ref,
                source_id,
                native_session_id,
                db_path,
                retry_argv,
            } => self.session_not_found_envelope(
                session_ref,
                source_id,
                native_session_id,
                db_path,
                retry_argv,
            ),
            Self::AnchorNotFound(payload) => self.anchor_not_found_envelope(
                &payload.session_ref,
                &payload.source_id,
                &payload.native_session_id,
                &payload.query,
                &payload.matched_profile_fields,
                &payload.read_page_argv,
            ),
            Self::ColdRoot { message }
            | Self::InvalidSelector { message, .. }
            | Self::IndexFailure { message }
            | Self::InvalidArguments { message }
            | Self::Output { message } => json!({"code": self.code(), "message": message}),
            Self::CommandFailedSilent => {
                json!({"code": self.code(), "message": self.message()})
            }
        };
        json!({"error": error})
    }

    fn index_unavailable_envelope(&self, db_path: &str, cwd: &str, default_root: &str) -> Value {
        let cwd_arg = serde_json::to_string(cwd).unwrap_or_else(|_| format!("\"{cwd}\""));
        json!({
            "code": self.code(),
            "message": self.message(),
            "dbPath": db_path,
            "hint": format!(
                "Run `shlog sync` first to create the default Codex index. Only for explicitly current-project questions, run `shlog sync --cwd {cwd_arg}` instead. No separate init command is needed; sync initializes and updates it."
            ),
            "nextAction": {
                "kind": "bootstrap_index",
                "reason": "index_unavailable",
                "commands": [
                    {
                        "label": "default Codex history",
                        "when": "first install or unscoped history query",
                        "recommended": true,
                        "argv": ["shlog", "sync"],
                        "selector": {"kind": "all", "source": "codex", "root": default_root},
                    },
                    {
                        "label": "current working directory only",
                        "when": "question is explicitly scoped to the current working directory",
                        "recommended": false,
                        "argv": ["shlog", "sync", "--cwd", cwd],
                        "selector": {"kind": "cwd", "source": "codex", "root": default_root, "cwd": cwd},
                    }
                ]
            }
        })
    }

    fn session_not_found_envelope(
        &self,
        session_ref: &str,
        source_id: &str,
        native_session_id: &str,
        db_path: &str,
        retry_argv: &[String],
    ) -> Value {
        json!({
            "code": self.code(),
            "message": self.message(),
            "sessionRef": session_ref,
            "sourceId": source_id,
            "nativeSessionId": native_session_id,
            "hint": format!(
                "Sherlog only reads indexed sessions. The raw session may exist but not be synced yet, or the id/source may not match this index. Run `shlog status --source {source_id} --json`; if coverage is missing or stale, run `shlog sync --source {source_id}`, then retry."
            ),
            "nextAction": {
                "kind": "check_coverage_then_retry_read",
                "reason": "session_not_found",
                "steps": [
                    format!("Verify that {session_ref} is the right sessionRef and source. If needed, use a source-qualified ref such as {source_id}:{native_session_id}."),
                    format!("Run shlog status --source {source_id} --json to check index freshness."),
                    format!("If status reports missing or stale coverage, run shlog sync --source {source_id} and retry the read command."),
                ],
                "commands": [
                    {
                        "label": "check source coverage",
                        "recommended": true,
                        "argv": ["shlog", "status", "--source", source_id, "--db", db_path, "--json"],
                    },
                    {
                        "label": "refresh default source index",
                        "recommended": false,
                        "argv": ["shlog", "sync", "--source", source_id, "--db", db_path],
                    },
                    {
                        "label": "retry read command",
                        "recommended": false,
                        "argv": retry_argv,
                    }
                ]
            }
        })
    }

    fn anchor_not_found_envelope(
        &self,
        session_ref: &str,
        source_id: &str,
        native_session_id: &str,
        query: &str,
        matched_profile_fields: &[String],
        read_page_argv: &[String],
    ) -> Value {
        let hint = if matched_profile_fields.is_empty() {
            "No message in this session matched the query. This can happen when the query targets session-level fields or when the session projection lacks the term; read the session projection or refine the query to message terms."
                .to_owned()
        } else {
            format!(
                "The query matched only session-level fields ({}) of this session. There is no message anchor for it; read the session projection (read-page) or refine the query to terms that appear in messages.",
                matched_profile_fields.join(", ")
            )
        };
        json!({
            "code": self.code(),
            "message": self.message(),
            "sessionRef": session_ref,
            "sourceId": source_id,
            "nativeSessionId": native_session_id,
            "query": query,
            "matchedProfileFields": matched_profile_fields,
            "hint": hint,
            "nextAction": {
                "kind": "read_session_projection",
                "reason": "anchor_not_found",
                "steps": [
                    format!("Read the session projection of {session_ref} with read-page to locate the evidence manually."),
                    "Refine the query to terms that appear in messages and retry read-range."
                ],
                "commands": [
                    {
                        "label": "read session projection",
                        "recommended": true,
                        "argv": read_page_argv,
                    }
                ]
            }
        })
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code(), self.message())
    }
}

impl std::error::Error for AppError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_error_has_stable_typed_shape() {
        let error = AppError::unsupported("find");
        let value = error.envelope();
        assert_eq!(value["error"]["code"], "unsupported_operation");
        assert_eq!(value["error"]["operation"], "find");
        assert_eq!(error.exit_code(), EXIT_FAILURE);
    }

    #[test]
    fn unsupported_source_matches_published_stdout_envelope() {
        let error = AppError::unsupported_source("  future  ");
        let value = error.envelope();
        assert_eq!(value["error"]["code"], "unsupported_source");
        assert_eq!(value["error"]["source"], "future");
        assert!(value["error"].get("operation").is_none());
        assert!(error.json_uses_stdout());
        assert_eq!(error.exit_code(), EXIT_FAILURE);
    }

    #[test]
    fn index_unavailable_has_actionable_bootstrap_commands() {
        let value = AppError::index_unavailable("/state/index.sqlite", "/repo", "/raw").envelope();
        assert_eq!(value["error"]["code"], "index_unavailable");
        assert_eq!(
            value["error"]["nextAction"]["commands"][0]["argv"],
            json!(["shlog", "sync"])
        );
        assert_eq!(
            value["error"]["nextAction"]["commands"][1]["selector"]["cwd"],
            "/repo"
        );
    }
}
