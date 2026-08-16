use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use blake3::Hasher;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::cold::ColdRootEntry;
use crate::config::INDEX_VERSION;
use crate::index::{ANALYZER_EPOCH, COVERAGE_EPOCH, PROJECTION_EPOCH, SCHEMA_VERSION};
use crate::tokenizer::query_terms;

use super::error::{MigrationError, MigrationResult};
use super::legacy::V7Fingerprint;

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct VerificationReport {
    pub session_count: u64,
    pub message_count: u64,
    pub document_count: u64,
    pub fts_row_count: u64,
    pub source_file_count: u64,
    pub cold_root_count: u64,
    pub representative_fts_checks: u64,
}

pub(super) fn verify_v8_copy(
    v7_path: &Path,
    v8_path: &Path,
    source: &V7Fingerprint,
    expected_cold_roots: &[ColdRootEntry],
) -> MigrationResult<VerificationReport> {
    let v7 = open_read_only(v7_path)?;
    let v8 = open_read_only(v8_path)?;

    let integrity: String = v8.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    require(
        integrity == "ok",
        format!("integrity_check returned {integrity:?}"),
    )?;
    let foreign_key_violation = v8
        .query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
        .optional()?;
    require(
        foreign_key_violation.is_none(),
        "foreign_key_check found a violation".to_owned(),
    )?;

    let user_version: i64 = v8.pragma_query_value(None, "user_version", |row| row.get(0))?;
    require(
        user_version == i64::from(SCHEMA_VERSION),
        format!("expected user_version {SCHEMA_VERSION}, got {user_version}"),
    )?;
    for (key, expected) in [
        ("schema_version", i64::from(SCHEMA_VERSION)),
        ("projection_epoch", PROJECTION_EPOCH),
        ("analyzer_epoch", ANALYZER_EPOCH),
        ("coverage_epoch", COVERAGE_EPOCH),
    ] {
        let value: String = v8.query_row("SELECT value FROM meta WHERE key=?", [key], |row| {
            row.get(0)
        })?;
        require(
            value.parse::<i64>().ok() == Some(expected),
            format!("meta {key} expected {expected}, got {value:?}"),
        )?;
    }

    let session_count = count(&v8, "sessions")?;
    let message_count: u64 = non_negative(v8.query_row(
        "SELECT COUNT(*) FROM documents WHERE kind='message'",
        [],
        |row| row.get::<_, i64>(0),
    )?)?;
    let profile_count: u64 = non_negative(v8.query_row(
        "SELECT COUNT(*) FROM documents WHERE kind='session_profile'",
        [],
        |row| row.get::<_, i64>(0),
    )?)?;
    let document_count = count(&v8, "documents")?;
    let fts_row_count = count(&v8, "documents_fts")?;
    let source_file_count = count(&v8, "source_files")?;
    let coverage_count = count(&v8, "coverage")?;
    let cold_root_count = count(&v8, "cold_roots")?;

    require(
        session_count == source.session_count,
        format!(
            "session count changed: v7={} v8={session_count}",
            source.session_count
        ),
    )?;
    require(
        message_count == source.message_count,
        format!(
            "message count changed: v7={} v8={message_count}",
            source.message_count
        ),
    )?;
    require(
        profile_count == session_count,
        format!(
            "expected one profile per session: sessions={session_count} profiles={profile_count}"
        ),
    )?;
    require(
        document_count == message_count + profile_count,
        format!(
            "document count mismatch: total={document_count} messages={message_count} profiles={profile_count}"
        ),
    )?;
    require(
        fts_row_count == document_count,
        format!("FTS row count {fts_row_count} != document count {document_count}"),
    )?;
    require(
        source_file_count == source.source_file_count,
        format!(
            "source-file metadata count changed: v7={} v8={source_file_count}",
            source.source_file_count
        ),
    )?;
    require(
        coverage_count == 0,
        format!("v7 coverage must become unknown, but v8 contains {coverage_count} rows"),
    )?;
    verify_cold_roots(&v8, expected_cold_roots)?;

    let invalid_locators: i64 = v8.query_row(
        "SELECT COUNT(*) FROM documents WHERE raw_start IS NOT NULL OR raw_end IS NOT NULL",
        [],
        |row| row.get(0),
    )?;
    require(
        invalid_locators == 0,
        format!("{invalid_locators} migrated documents retained unproved raw locators"),
    )?;
    let forged_cursors: i64 = v8.query_row(
        "SELECT COUNT(*) FROM source_files WHERE indexed_bytes<>0 OR head_digest<>'' OR \
           boundary_digest<>'' OR next_seq<>0 OR reducer_checkpoint IS NOT NULL OR \
           source_generation<>'' OR mtime_ns IS NOT NULL",
        [],
        |row| row.get(0),
    )?;
    require(
        forged_cursors == 0,
        format!(
            "{forged_cursors} migrated source files claim unproved append cursors or exact mtimes"
        ),
    )?;
    let wrong_document_epochs: i64 = v8.query_row(
        "SELECT COUNT(*) FROM documents WHERE projection_epoch<>?",
        [PROJECTION_EPOCH],
        |row| row.get(0),
    )?;
    require(
        wrong_document_epochs == 0,
        format!("{wrong_document_epochs} documents have the wrong projection epoch"),
    )?;
    let wrong_source_epochs: i64 = v8.query_row(
        "SELECT COUNT(*) FROM source_files WHERE projection_epoch<>? OR analyzer_epoch<>? OR coverage_epoch<>?",
        (PROJECTION_EPOCH, ANALYZER_EPOCH, COVERAGE_EPOCH),
        |row| row.get(0),
    )?;
    require(
        wrong_source_epochs == 0,
        format!("{wrong_source_epochs} source files have the wrong epochs"),
    )?;
    verify_migration_receipt(&v8, source, expected_cold_roots.len())?;

    let count_mismatches: i64 = v8.query_row(
        "SELECT COUNT(*) FROM sessions s WHERE \
           s.message_count != (SELECT COUNT(*) FROM documents d \
                               WHERE d.session_id=s.id AND d.kind='message') OR \
           s.document_count != (SELECT COUNT(*) FROM documents d WHERE d.session_id=s.id)",
        [],
        |row| row.get(0),
    )?;
    require(
        count_mismatches == 0,
        format!("{count_mismatches} session count summaries are inconsistent"),
    )?;
    let profile_mismatches: i64 = v8.query_row(
        "SELECT COUNT(*) FROM sessions s JOIN documents d ON d.session_id=s.id \
         WHERE d.kind='session_profile' AND (d.title_text<>s.title OR \
           d.summary_text<>s.summary_text OR d.compact_text<>s.compact_text OR \
           d.reasoning_text<>s.reasoning_summary_text OR d.raw_start IS NOT NULL OR \
           d.raw_end IS NOT NULL)",
        [],
        |row| row.get(0),
    )?;
    require(
        profile_mismatches == 0,
        format!("{profile_mismatches} generated profiles do not match session projection"),
    )?;

    let v7_projection = projection_digest_v7(&v7)?;
    let v8_projection = projection_digest_v8(&v8)?;
    require(
        v7_projection == v8_projection,
        format!("stored projection digest changed: v7={v7_projection} v8={v8_projection}"),
    )?;
    let v7_source_files = source_file_digest_v7(&v7)?;
    let v8_source_files = source_file_digest_v8(&v8)?;
    require(
        v7_source_files == v8_source_files,
        format!("source-file metadata digest changed: v7={v7_source_files} v8={v8_source_files}"),
    )?;

    let representative_fts_checks = verify_representative_fts(&v8)?;
    Ok(VerificationReport {
        session_count,
        message_count,
        document_count,
        fts_row_count,
        source_file_count,
        cold_root_count,
        representative_fts_checks,
    })
}

fn verify_cold_roots(connection: &Connection, expected: &[ColdRootEntry]) -> MigrationResult<()> {
    let mut statement = connection
        .prepare("SELECT source_id, root, added_at FROM cold_roots ORDER BY source_id, root")?;
    let actual = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut expected = expected
        .iter()
        .map(|entry| {
            (
                entry.source_id.clone(),
                entry.root.clone(),
                entry.added_at.clone(),
            )
        })
        .collect::<Vec<_>>();
    expected.sort();
    require(
        actual == expected,
        format!("cold-root rows differ: expected={expected:?} actual={actual:?}"),
    )
}

fn verify_migration_receipt(
    connection: &Connection,
    source: &V7Fingerprint,
    cold_root_count: usize,
) -> MigrationResult<()> {
    let encoded: String = connection.query_row(
        "SELECT value FROM meta WHERE key='migration_receipt'",
        [],
        |row| row.get(0),
    )?;
    let receipt: serde_json::Value = serde_json::from_str(&encoded).map_err(|error| {
        MigrationError::Verification(format!("migration receipt is invalid JSON: {error}"))
    })?;
    let expected = serde_json::json!({
        "fromSchemaVersion": 7,
        "toSchemaVersion": SCHEMA_VERSION,
        "sourceFingerprint": source.digest,
        "sessionCount": source.session_count,
        "messageCount": source.message_count,
        "sourceFileCount": source.source_file_count,
        "coldRootCount": cold_root_count,
        "coverageDisposition": "cleared",
        "coverageRowsCleared": source.coverage_count,
        "indexVersion": INDEX_VERSION,
    });
    require(
        receipt == expected,
        format!("migration receipt differs: expected={expected} actual={receipt}"),
    )
}

fn projection_digest_v7(connection: &Connection) -> MigrationResult<String> {
    digest_queries(
        connection,
        &[
            "SELECT source_id, native_session_id, session_key, session_uuid, file_path, \
                    source_root, title, summary_text, compact_text, reasoning_summary_text, \
                    cwd, model, started_at, ended_at, path_date, \
                    CAST(ROUND(raw_file_mtime) AS INTEGER), raw_file_size \
             FROM sessions ORDER BY source_id, native_session_id",
            "SELECT s.source_id, s.native_session_id, m.seq, m.role, m.content_text, \
                    m.timestamp, m.source_kind, m.session_uuid \
             FROM messages m JOIN sessions s ON s.id=m.session_id \
             ORDER BY s.source_id, s.native_session_id, m.seq",
        ],
    )
}

fn projection_digest_v8(connection: &Connection) -> MigrationResult<String> {
    digest_queries(
        connection,
        &[
            "SELECT source_id, native_session_id, session_key, session_uuid, file_path, \
                    source_root, title, summary_text, compact_text, reasoning_summary_text, \
                    cwd, model, started_at, ended_at, path_date, raw_file_mtime, raw_file_size \
             FROM sessions ORDER BY source_id, native_session_id",
            "SELECT s.source_id, s.native_session_id, d.seq, d.role, d.body_text, \
                    d.timestamp, d.source_kind, s.session_uuid \
             FROM documents d JOIN sessions s ON s.id=d.session_id \
             WHERE d.kind='message' \
             ORDER BY s.source_id, s.native_session_id, d.seq",
        ],
    )
}

fn source_file_digest_v7(connection: &Connection) -> MigrationResult<String> {
    if !table_exists(connection, "source_file_meta_cache")?
        || count(connection, "source_file_meta_cache")? == 0
    {
        return Ok(blake3::hash(b"missing-source-files").to_hex().to_string());
    }
    digest_queries(
        connection,
        &[
            "SELECT source_id, file_path, mtime_ms, size, cwd, path_date, extra_fingerprint \
           FROM source_file_meta_cache ORDER BY source_id, file_path",
        ],
    )
}

fn source_file_digest_v8(connection: &Connection) -> MigrationResult<String> {
    if count(connection, "source_files")? == 0 {
        return Ok(blake3::hash(b"missing-source-files").to_hex().to_string());
    }
    digest_queries(
        connection,
        &[
            "SELECT source_id, file_path, mtime_ms, size, cwd, path_date, extra_fingerprint \
           FROM source_files ORDER BY source_id, file_path",
        ],
    )
}

fn verify_representative_fts(connection: &Connection) -> MigrationResult<u64> {
    const FIELDS: [&str; 5] = [
        "body_text",
        "title_text",
        "summary_text",
        "compact_text",
        "reasoning_text",
    ];
    let mut checked = 0_u64;
    let mut sampled = HashSet::new();
    for field in FIELDS {
        let count_sql = format!("SELECT COUNT(*) FROM documents WHERE length({field})>0");
        let field_count: i64 = connection.query_row(&count_sql, [], |row| row.get(0))?;
        if field_count <= 0 {
            continue;
        }
        for offset in [0, field_count / 2, field_count - 1] {
            let sample_sql = format!(
                "SELECT id, {field} FROM documents WHERE length({field})>0 \
                 ORDER BY id LIMIT 1 OFFSET ?"
            );
            let (document_id, text): (i64, String) =
                connection
                    .query_row(&sample_sql, [offset], |row| Ok((row.get(0)?, row.get(1)?)))?;
            if !sampled.insert((field, document_id)) {
                continue;
            }
            let Some(term) = query_terms(&text).into_iter().next() else {
                continue;
            };
            let fts_query = format!("{field}: \"{}\"", term.replace('"', "\"\""));
            let found: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM documents_fts \
                 WHERE documents_fts MATCH ? AND rowid=?)",
                (&fts_query, document_id),
                |row| row.get(0),
            )?;
            require(
                found,
                format!(
                    "document {document_id} field {field} is absent from FTS for token {term:?}"
                ),
            )?;
            checked = checked.checked_add(1).ok_or_else(|| {
                MigrationError::Verification("representative FTS count overflow".to_owned())
            })?;
        }
    }
    Ok(checked)
}

fn digest_queries(connection: &Connection, queries: &[&str]) -> MigrationResult<String> {
    let mut hasher = Hasher::new();
    for sql in queries {
        let mut statement = connection.prepare(sql)?;
        let column_count = statement.column_count();
        let mut rows = statement.query([])?;
        let mut count = 0_u64;
        while let Some(row) = rows.next()? {
            hasher.update(b"R");
            for index in 0..column_count {
                hash_value(&mut hasher, row.get_ref(index)?);
            }
            count += 1;
        }
        hasher.update(&count.to_le_bytes());
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn hash_value(hasher: &mut Hasher, value: ValueRef<'_>) {
    match value {
        ValueRef::Null => {
            hasher.update(b"N");
        }
        ValueRef::Integer(value) => {
            hasher.update(b"I");
            hasher.update(&value.to_le_bytes());
        }
        ValueRef::Real(value) => {
            hasher.update(b"F");
            hasher.update(&value.to_bits().to_le_bytes());
        }
        ValueRef::Text(value) => {
            hasher.update(b"T");
            hash_bytes(hasher, value);
        }
        ValueRef::Blob(value) => {
            hasher.update(b"B");
            hash_bytes(hasher, value);
        }
    }
}

fn hash_bytes(hasher: &mut Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn open_read_only(path: &Path) -> MigrationResult<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    connection.pragma_update(None, "query_only", "ON")?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    Ok(connection)
}

fn count(connection: &Connection, table: &str) -> MigrationResult<u64> {
    non_negative(
        connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get::<_, i64>(0)
        })?,
    )
}

fn non_negative(value: i64) -> MigrationResult<u64> {
    u64::try_from(value).map_err(|_| {
        MigrationError::Verification(format!("SQLite returned negative count {value}"))
    })
}

fn table_exists(connection: &Connection, table: &str) -> MigrationResult<bool> {
    Ok(connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=? LIMIT 1",
            [table],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn require(condition: bool, detail: String) -> MigrationResult<()> {
    if condition {
        Ok(())
    } else {
        Err(MigrationError::Verification(detail))
    }
}
