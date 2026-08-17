//! Public source and session identity rules.

use std::fmt;
use std::str::FromStr;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

pub const DEFAULT_SOURCE_ID: SourceId = SourceId::Codex;

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Deserialize,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
    Serialize,
    ValueEnum,
)]
#[serde(rename_all = "kebab-case")]
#[value(rename_all = "kebab-case")]
pub enum SourceId {
    #[default]
    Codex,
    ClaudeCode,
    Pi,
    Dsh,
}

impl SourceId {
    pub const ALL: [Self; 4] = [Self::Codex, Self::ClaudeCode, Self::Pi, Self::Dsh];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude-code",
            Self::Pi => "pi",
            Self::Dsh => "dsh",
        }
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for SourceId {
    type Err = UnsupportedSourceId;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "codex" => Ok(Self::Codex),
            "claude-code" => Ok(Self::ClaudeCode),
            "pi" => Ok(Self::Pi),
            "dsh" => Ok(Self::Dsh),
            _ => Err(UnsupportedSourceId(value.to_owned())),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnsupportedSourceId(pub String);

impl fmt::Display for UnsupportedSourceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unsupported source {:?}; expected codex, claude-code, pi, or dsh",
            self.0
        )
    }
}

impl std::error::Error for UnsupportedSourceId {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRef {
    pub source_id: SourceId,
    pub native_session_id: String,
}

impl SessionRef {
    pub fn qualified(&self) -> String {
        format!("{}:{}", self.source_id, self.native_session_id)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionIdentity {
    pub source_id: SourceId,
    pub native_session_id: String,
    pub session_key: String,
}

impl SessionIdentity {
    pub fn new(source_id: SourceId, native_session_id: impl Into<String>) -> Self {
        let native_session_id = native_session_id.into();
        let session_key = format!("{source_id}:{native_session_id}");
        Self {
            source_id,
            native_session_id,
            session_key,
        }
    }

    /// Apply the legacy-compatible identity defaults used by indexed sessions.
    pub fn from_session_fields(
        source_id: Option<SourceId>,
        native_session_id: Option<&str>,
        session_uuid: &str,
        session_key: Option<&str>,
    ) -> Self {
        let source_id = source_id.unwrap_or(DEFAULT_SOURCE_ID);
        let native_session_id = native_session_id.unwrap_or(session_uuid).to_owned();
        let session_key = session_key
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{source_id}:{native_session_id}"));
        Self {
            source_id,
            native_session_id,
            session_key,
        }
    }

    pub fn as_session_ref(&self) -> SessionRef {
        SessionRef {
            source_id: self.source_id,
            native_session_id: self.native_session_id.clone(),
        }
    }
}

/// Parse only recognized source prefixes. Unknown prefixes remain part of a
/// bare Codex native ID, matching the published CLI contract.
pub fn parse_session_ref(value: &str) -> SessionRef {
    if let Some((prefix, native_session_id)) = value.split_once(':')
        && let Ok(source_id) = prefix.parse::<SourceId>()
    {
        return SessionRef {
            source_id,
            native_session_id: native_session_id.to_owned(),
        };
    }

    SessionRef {
        source_id: DEFAULT_SOURCE_ID,
        native_session_id: value.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_ids_round_trip_through_text_and_json() {
        for source in SourceId::ALL {
            assert_eq!(source.as_str().parse::<SourceId>(), Ok(source));
            assert_eq!(
                serde_json::from_str::<SourceId>(&serde_json::to_string(&source).unwrap()).unwrap(),
                source
            );
        }
        assert_eq!(
            serde_json::to_string(&SourceId::ClaudeCode).unwrap(),
            r#""claude-code""#
        );
    }

    #[test]
    fn defaults_legacy_session_fields_to_codex_uuid() {
        let identity = SessionIdentity::from_session_fields(None, None, "uuid-1", None);
        assert_eq!(identity, SessionIdentity::new(SourceId::Codex, "uuid-1"));
        assert_eq!(
            serde_json::to_string(&identity).unwrap(),
            r#"{"sourceId":"codex","nativeSessionId":"uuid-1","sessionKey":"codex:uuid-1"}"#
        );
    }

    #[test]
    fn same_native_id_cannot_collide_across_sources() {
        let identities = SourceId::ALL.map(|source| SessionIdentity::new(source, "shared"));
        assert_eq!(identities[0].session_key, "codex:shared");
        assert_eq!(identities[1].session_key, "claude-code:shared");
        assert_eq!(identities[2].session_key, "pi:shared");
        assert_ne!(identities[0].session_key, identities[1].session_key);
        assert_ne!(identities[1].session_key, identities[2].session_key);
    }

    #[test]
    fn separators_inside_native_ids_do_not_create_key_collisions() {
        let codex = SessionIdentity::new(SourceId::Codex, "claude-code:same");
        let claude = SessionIdentity::new(SourceId::ClaudeCode, "same");
        assert_eq!(codex.session_key, "codex:claude-code:same");
        assert_eq!(claude.session_key, "claude-code:same");
        assert_ne!(codex.session_key, claude.session_key);

        assert_eq!(
            parse_session_ref(&codex.session_key),
            codex.as_session_ref()
        );
        assert_eq!(
            parse_session_ref(&claude.session_key),
            claude.as_session_ref()
        );
    }

    #[test]
    fn parser_splits_only_the_first_recognized_prefix() {
        assert_eq!(
            parse_session_ref("claude-code:project:session"),
            SessionRef {
                source_id: SourceId::ClaudeCode,
                native_session_id: "project:session".to_owned(),
            }
        );
        assert_eq!(
            parse_session_ref("future:project:session"),
            SessionRef {
                source_id: SourceId::Codex,
                native_session_id: "future:project:session".to_owned(),
            }
        );
        assert_eq!(
            parse_session_ref(":leading"),
            SessionRef {
                source_id: SourceId::Codex,
                native_session_id: ":leading".to_owned(),
            }
        );
    }

    #[test]
    fn explicit_legacy_identity_fields_are_preserved() {
        let identity = SessionIdentity::from_session_fields(
            Some(SourceId::Pi),
            Some("native"),
            "compat-uuid",
            Some("custom-key"),
        );
        assert_eq!(identity.source_id, SourceId::Pi);
        assert_eq!(identity.native_session_id, "native");
        assert_eq!(identity.session_key, "custom-key");
    }
}
