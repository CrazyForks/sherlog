use rusqlite::{Connection, OptionalExtension};

use crate::config::INDEX_VERSION;

use super::{
    ANALYZER_EPOCH, COVERAGE_EPOCH, IndexError, IndexLayout, IndexMetadata, IndexResult,
    PROJECTION_EPOCH, SCHEMA_VERSION,
};

pub(crate) const BUSY_TIMEOUT_MS: u64 = 5_000;

pub(crate) const V8_SCHEMA: &str = include_str!("v8.sql");

pub(crate) fn initialize_v8(connection: &mut Connection) -> IndexResult<()> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(V8_SCHEMA)?;
    let created_at: String =
        transaction.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
            row.get(0)
        })?;
    let entries = [
        ("schema_version", SCHEMA_VERSION.to_string()),
        ("projection_epoch", PROJECTION_EPOCH.to_string()),
        ("analyzer_epoch", ANALYZER_EPOCH.to_string()),
        ("coverage_epoch", COVERAGE_EPOCH.to_string()),
        ("index_version", INDEX_VERSION.to_owned()),
        ("created_at", created_at),
    ];
    for (key, value) in entries {
        transaction.execute("INSERT INTO meta(key, value) VALUES (?, ?)", (key, value))?;
    }
    transaction.commit()?;
    Ok(())
}

pub(crate) fn detect_layout(connection: &Connection) -> IndexResult<IndexLayout> {
    let user_version: i32 =
        connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let sessions_kind = schema_object_type(connection, "sessions")?;
    let messages_are_table = object_is(connection, "messages", "table")?;
    let documents_are_table = object_is(connection, "documents", "table")?;
    let session_rows_are_table = object_is(connection, "session_rows", "table")?;

    if user_version == SCHEMA_VERSION {
        if sessions_kind.as_deref() == Some("view") && session_rows_are_table && documents_are_table
        {
            validate_v8_shape(connection, user_version)?;
            return Ok(IndexLayout::V8);
        }
        return Err(IndexError::unsupported(
            user_version,
            "user_version=8 requires a physical session_rows table and a read-only sessions view",
        ));
    }
    if user_version > SCHEMA_VERSION {
        return Err(IndexError::unsupported(
            user_version,
            "index was created by a newer Sherlog schema",
        ));
    }
    if sessions_kind.as_deref() == Some("table") && messages_are_table {
        validate_v7_shape(connection, user_version)?;
        return Ok(IndexLayout::V7);
    }

    Err(IndexError::unsupported(
        user_version,
        "expected v7 sessions/messages or v8 sessions/documents schema",
    ))
}

pub(crate) fn read_metadata(
    connection: &Connection,
    layout: IndexLayout,
) -> IndexResult<IndexMetadata> {
    if layout == IndexLayout::V7 {
        return Ok(IndexMetadata {
            schema_version: 7,
            projection_epoch: 0,
            analyzer_epoch: 0,
            coverage_epoch: 0,
            index_version: "shlog-v7-source-identity".to_owned(),
            created_at: String::new(),
            upgraded_at: None,
            migration_receipt: None,
        });
    }

    Ok(IndexMetadata {
        schema_version: meta_i64(connection, "schema_version")? as i32,
        projection_epoch: meta_i64(connection, "projection_epoch")?,
        analyzer_epoch: meta_i64(connection, "analyzer_epoch")?,
        coverage_epoch: meta_i64(connection, "coverage_epoch")?,
        index_version: meta_required(connection, "index_version")?,
        created_at: meta_required(connection, "created_at")?,
        upgraded_at: meta_optional(connection, "upgraded_at")?,
        migration_receipt: meta_optional(connection, "migration_receipt")?,
    })
}

/// Writers require every persisted compatibility axis to match the binary.
/// Readers deliberately omit `coverage_epoch` from this gate so an old
/// coverage projection cannot make otherwise compatible stored content
/// unreadable; `IndexReader::coverage_records` hides those stale rows instead.
pub(crate) fn validate_v8_writer_metadata(connection: &Connection) -> IndexResult<()> {
    let metadata = read_metadata(connection, IndexLayout::V8)?;
    validate_v8_content_metadata(SCHEMA_VERSION, &metadata)?;
    if metadata.coverage_epoch != COVERAGE_EPOCH {
        return Err(IndexError::unsupported(
            SCHEMA_VERSION,
            format!(
                "meta coverage_epoch is {}, but this writer requires {COVERAGE_EPOCH}",
                metadata.coverage_epoch
            ),
        ));
    }
    Ok(())
}

pub(crate) fn table_exists(connection: &Connection, table: &str) -> IndexResult<bool> {
    Ok(schema_object_type(connection, table)?.is_some())
}

fn validate_v8_shape(connection: &Connection, user_version: i32) -> IndexResult<()> {
    for table in [
        "meta",
        "session_rows",
        "source_files",
        "documents",
        "documents_fts",
        "coverage",
        "cold_roots",
    ] {
        if !object_is(connection, table, "table")? {
            return Err(IndexError::unsupported(
                user_version,
                format!("v8 table {table} is missing"),
            ));
        }
    }
    if !object_is(connection, "sessions", "view")? {
        return Err(IndexError::unsupported(
            user_version,
            "v8 sessions must be the public compatibility view",
        ));
    }
    if !column_exists(connection, "source_files", "mtime_ns")? {
        return Err(IndexError::unsupported(
            user_version,
            "v8 source_files.mtime_ns is required for exact cache identity",
        ));
    }
    for column in [
        "id",
        "source_id",
        "native_session_id",
        "session_key",
        "session_uuid",
        "file_path",
        "source_root",
        "title",
        "summary_text",
        "compact_text",
        "reasoning_summary_text",
        "cwd",
        "model",
        "started_at",
        "ended_at",
        "path_date",
        "message_count",
        "document_count",
        "raw_file_mtime",
        "raw_file_size",
        "index_version",
        "updated_at",
    ] {
        if !column_exists(connection, "sessions", column)? {
            return Err(IndexError::unsupported(
                user_version,
                format!("v8 sessions view is missing stable column {column}"),
            ));
        }
    }
    let write_triggers: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' AND tbl_name='sessions'",
        [],
        |row| row.get(0),
    )?;
    if write_triggers != 0 {
        return Err(IndexError::unsupported(
            user_version,
            "v8 sessions view must not have write-through triggers",
        ));
    }
    let metadata = read_metadata(connection, IndexLayout::V8)?;
    validate_v8_content_metadata(user_version, &metadata)
}

fn validate_v8_content_metadata(user_version: i32, metadata: &IndexMetadata) -> IndexResult<()> {
    if metadata.schema_version != SCHEMA_VERSION {
        return Err(IndexError::unsupported(
            user_version,
            format!("meta schema_version is {}", metadata.schema_version),
        ));
    }
    if metadata.projection_epoch != PROJECTION_EPOCH {
        return Err(IndexError::unsupported(
            user_version,
            format!(
                "meta projection_epoch is {}, but this reader requires {PROJECTION_EPOCH}",
                metadata.projection_epoch
            ),
        ));
    }
    if metadata.analyzer_epoch != ANALYZER_EPOCH {
        return Err(IndexError::unsupported(
            user_version,
            format!(
                "meta analyzer_epoch is {}, but this reader requires {ANALYZER_EPOCH}",
                metadata.analyzer_epoch
            ),
        ));
    }
    if metadata.index_version != INDEX_VERSION {
        return Err(IndexError::unsupported(
            user_version,
            format!(
                "meta index_version is {:?}, but this reader requires {INDEX_VERSION:?}",
                metadata.index_version
            ),
        ));
    }
    Ok(())
}

fn schema_object_type(connection: &Connection, name: &str) -> IndexResult<Option<String>> {
    Ok(connection
        .query_row(
            "SELECT type FROM sqlite_master WHERE name=? LIMIT 1",
            [name],
            |row| row.get(0),
        )
        .optional()?)
}

fn object_is(connection: &Connection, name: &str, expected_type: &str) -> IndexResult<bool> {
    Ok(schema_object_type(connection, name)?.as_deref() == Some(expected_type))
}

fn validate_v7_shape(connection: &Connection, user_version: i32) -> IndexResult<()> {
    for column in ["source_id", "native_session_id", "session_key"] {
        if !column_exists(connection, "sessions", column)? {
            return Err(IndexError::unsupported(
                user_version,
                format!("v7 sessions.{column} is missing"),
            ));
        }
    }
    if table_exists(connection, "coverage")? && !column_exists(connection, "coverage", "source_id")?
    {
        return Err(IndexError::unsupported(
            user_version,
            "v7 coverage.source_id is missing",
        ));
    }
    Ok(())
}

fn column_exists(connection: &Connection, table: &str, column: &str) -> IndexResult<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        if row.get::<_, String>(1)? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn meta_required(connection: &Connection, key: &str) -> IndexResult<String> {
    meta_optional(connection, key)?
        .ok_or_else(|| IndexError::InvalidData(format!("required meta key {key:?} is missing")))
}

fn meta_optional(connection: &Connection, key: &str) -> IndexResult<Option<String>> {
    Ok(connection
        .query_row("SELECT value FROM meta WHERE key = ?", [key], |row| {
            row.get(0)
        })
        .optional()?)
}

fn meta_i64(connection: &Connection, key: &str) -> IndexResult<i64> {
    let value = meta_required(connection, key)?;
    value.parse().map_err(|_| {
        IndexError::InvalidData(format!("meta key {key:?} is not an integer: {value:?}"))
    })
}
