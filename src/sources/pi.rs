use std::path::Path;

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::identity::SourceId;
use crate::model::{MessageRole, SourceKind};

use super::catalog::AcceptedMetadata;
use super::jsonl::{object, read_bounded_lines, scan_json_records, string, timestamp_date};
use super::{
    DocumentKind, EmptyProjection, FullProjectionReason, MessageProjection, ProjectedSource,
    ProjectionCheckpoint, ProjectionMode, ProjectionOutcome, SessionProjection, SourceDocument,
    SourceError, SourceFile, build_session_summary, fallback_session_id, fallback_timestamp,
    first_user_title, normalize_summary_text, time_range, truncate_chars,
};

#[derive(Debug)]
struct AcceptedSession {
    session_id: String,
    cwd: String,
    timestamp: String,
}

#[derive(Debug)]
struct AcceptedMessage {
    role: MessageRole,
    content_text: String,
    timestamp: String,
}

#[derive(Debug)]
struct AcceptedCompaction {
    summary_text: String,
    timestamp: String,
}

pub(crate) fn inventory_metadata(path: &Path) -> Result<Option<AcceptedMetadata>, SourceError> {
    let mut cwd = String::new();
    let mut path_date = None;
    let mut accepted_count = 0_usize;
    let mut hasher = Sha256::new();
    hasher.update(b"sherlog:pi:accepted:v1");

    scan_json_records(path, None, |record| {
        if let Some(session) = accepted_session(record) {
            if cwd.is_empty() {
                cwd = session.cwd.clone();
            }
            if path_date.is_none() {
                path_date = timestamp_date(&session.timestamp);
            }
            hash_fields(
                &mut hasher,
                "session",
                &[&session.session_id, &session.cwd, &session.timestamp],
            );
            return true;
        }
        if record.get("type").and_then(Value::as_str) == Some("model_change") {
            let model = string(record, "modelId");
            if !model.is_empty() {
                hash_fields(&mut hasher, "model_change", &[&model]);
            }
            return true;
        }
        if let Some(message) = accepted_message(record) {
            accepted_count += 1;
            if path_date.is_none() {
                path_date = timestamp_date(&message.timestamp);
            }
            hash_fields(
                &mut hasher,
                "message",
                &[
                    role_text(message.role),
                    &message.timestamp,
                    &message.content_text,
                ],
            );
            return true;
        }
        if let Some(compaction) = accepted_compaction(record) {
            accepted_count += 1;
            if path_date.is_none() {
                path_date = timestamp_date(&compaction.timestamp);
            }
            hash_fields(
                &mut hasher,
                "compaction",
                &[&compaction.timestamp, &compaction.summary_text],
            );
            return true;
        }
        true
    })?;

    Ok((accepted_count > 0).then(|| AcceptedMetadata {
        cwd,
        path_date,
        fingerprint: hex::encode(hasher.finalize()),
    }))
}

pub(crate) fn project(
    file: &SourceFile,
    read_limit: u64,
    checkpoint: Option<&ProjectionCheckpoint>,
) -> Result<ProjectionOutcome, SourceError> {
    if let Some(checkpoint) = checkpoint {
        return Ok(ProjectionOutcome::FullRequired {
            reason: if checkpoint.source_id == SourceId::Pi {
                FullProjectionReason::DeltaUnsupported
            } else {
                FullProjectionReason::SourceMismatch
            },
            read_proof: None,
        });
    }

    let mut documents = Vec::new();
    let mut session_id = String::new();
    let mut cwd = String::new();
    let mut model = String::new();
    let mut session_timestamp = String::new();
    let mut compaction_summaries = Vec::new();
    let mut compaction_timestamps = Vec::new();
    let read = read_bounded_lines(file, read_limit, 0, |record, raw_start, raw_end| {
        if let Some(session) = accepted_session(record) {
            if session_id.is_empty() && !session.session_id.is_empty() {
                session_id = session.session_id;
            }
            if cwd.is_empty() {
                cwd = session.cwd;
            }
            if session_timestamp.is_empty() {
                session_timestamp = session.timestamp;
            }
            return Ok(());
        }
        if record.get("type").and_then(Value::as_str) == Some("model_change") {
            let model_id = string(record, "modelId");
            if !model_id.is_empty() {
                model = model_id;
            }
            return Ok(());
        }
        if let Some(compaction) = accepted_compaction(record) {
            compaction_summaries.push(compaction.summary_text);
            if !compaction.timestamp.is_empty() {
                compaction_timestamps.push(compaction.timestamp);
            }
            return Ok(());
        }
        let Some(message) = accepted_message(record) else {
            return Ok(());
        };
        let seq = documents.len() as i64;
        documents.push(SourceDocument {
            kind: DocumentKind::Message,
            message: MessageProjection {
                role: message.role,
                content_text: message.content_text,
                timestamp: message.timestamp,
                seq,
                source_kind: SourceKind::EventMsg,
            },
            raw_start,
            raw_end,
        });
        Ok(())
    })?;

    if read.proof.opened.identity != file.identity {
        return Ok(ProjectionOutcome::FullRequired {
            reason: FullProjectionReason::FileIdentityChanged,
            read_proof: Some(read.proof),
        });
    }
    let checkpoint = ProjectionCheckpoint {
        source_id: SourceId::Pi,
        file_identity: read.proof.opened.identity.clone(),
        indexed_bytes: read.proof.safe_offset,
        prefix_digest: read.safe_prefix_digest,
        next_seq: documents.len() as i64,
        reducer_state: r#"{"version":1,"mode":"full_only"}"#.to_owned(),
    };
    let compact_text = truncate_chars(
        &compaction_summaries
            .iter()
            .map(|summary| normalize_summary_text(summary))
            .filter(|summary| !summary.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        20_000,
    );
    if documents.is_empty() && compact_text.is_empty() {
        return Ok(ProjectionOutcome::Skipped(EmptyProjection {
            read_proof: read.proof,
            checkpoint,
        }));
    }

    let native_session_id = if session_id.is_empty() {
        fallback_session_id(file)
    } else {
        session_id
    };
    let session_key = format!("pi:{native_session_id}");
    let fallback = fallback_timestamp(file);
    let additional = std::iter::once(session_timestamp)
        .chain(compaction_timestamps)
        .filter(|timestamp| !timestamp.is_empty());
    let (started_at, ended_at) = time_range(&documents, additional, &fallback);
    let title = first_user_title(&documents)
        .or_else(|| {
            compaction_summaries
                .iter()
                .map(|summary| truncate_chars(&normalize_summary_text(summary), 120))
                .find(|summary| !summary.is_empty())
        })
        .unwrap_or_else(|| "(no title)".to_owned());
    let session = SessionProjection {
        source_id: SourceId::Pi,
        native_session_id,
        session_key: session_key.clone(),
        session_uuid: session_key,
        file_path: file.file_path.to_string_lossy().into_owned(),
        title,
        summary_text: build_session_summary(&documents),
        compact_text,
        reasoning_summary_text: String::new(),
        cwd: if cwd.is_empty() {
            file.cwd.clone()
        } else {
            cwd
        },
        model,
        started_at,
        ended_at,
        document_count: documents.len() as u64,
    };
    Ok(ProjectionOutcome::Projected(Box::new(ProjectedSource {
        mode: ProjectionMode::Full,
        session,
        documents,
        read_proof: read.proof,
        checkpoint,
    })))
}

fn accepted_session(record: &Map<String, Value>) -> Option<AcceptedSession> {
    if record.get("type").and_then(Value::as_str) != Some("session") {
        return None;
    }
    let cwd = string(record, "cwd");
    let timestamp = string(record, "timestamp");
    if cwd.is_empty() || timestamp.is_empty() {
        return None;
    }
    Some(AcceptedSession {
        session_id: string(record, "id"),
        cwd,
        timestamp,
    })
}

fn accepted_message(record: &Map<String, Value>) -> Option<AcceptedMessage> {
    if record.get("type").and_then(Value::as_str) != Some("message") {
        return None;
    }
    let message = object(record, "message")?;
    let role = match message.get("role").and_then(Value::as_str) {
        Some("user") => MessageRole::User,
        Some("assistant") => MessageRole::Assistant,
        _ => return None,
    };
    let content_text = message
        .get("content")
        .map(text_from_content)
        .unwrap_or_default();
    if content_text.is_empty() {
        return None;
    }
    let timestamp = string(record, "timestamp");
    let timestamp = if timestamp.is_empty() {
        string(message, "timestamp")
    } else {
        timestamp
    };
    Some(AcceptedMessage {
        role,
        content_text,
        timestamp,
    })
}

fn accepted_compaction(record: &Map<String, Value>) -> Option<AcceptedCompaction> {
    if record.get("type").and_then(Value::as_str) != Some("compaction") {
        return None;
    }
    let summary_text = string(record, "summary");
    (!summary_text.is_empty()).then(|| AcceptedCompaction {
        summary_text,
        timestamp: string(record, "timestamp"),
    })
}

fn text_from_content(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.trim().to_owned();
    }
    let Some(items) = value.as_array() else {
        return String::new();
    };
    items
        .iter()
        .filter_map(|item| {
            if let Some(text) = item.as_str().map(str::trim).filter(|text| !text.is_empty()) {
                return Some(text);
            }
            let item = item.as_object()?;
            if item.get("type").and_then(Value::as_str) != Some("text") {
                return None;
            }
            item.get("text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn role_text(role: MessageRole) -> &'static str {
    match role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
    }
}

fn hash_fields(hasher: &mut Sha256, tag: &str, fields: &[&str]) {
    hash_value(hasher, tag);
    for field in fields {
        hash_value(hasher, field);
    }
}

fn hash_value(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}
