use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params, params_from_iter};

use crate::config::INDEX_VERSION;
use crate::identity::{SessionIdentity, SourceId};
use crate::model::MessageRole;
use crate::selector::{Selector, SelectorKind};
use crate::tokenizer::tokenized_text;

use super::reader::inspect_v8_invariants;
use super::schema::{BUSY_TIMEOUT_MS, detect_layout, initialize_v8, validate_v8_writer_metadata};
use super::{
    ANALYZER_EPOCH, COVERAGE_EPOCH, CommitReceipt, CoverageWrite, DocumentKind, IndexError,
    IndexLayout, IndexResult, MessageWrite, PROJECTION_EPOCH, PruneOutcome, SelectorCounts,
    SessionBundle, SessionProfileWrite, SessionWrite, SourceFileState, StoredDocument,
};

/// The single concrete SQLite writer. Opening a v7 index for writing is
/// deliberately rejected: copy migration will be a separate explicit flow.
pub struct IndexWriter {
    connection: Connection,
    path: PathBuf,
}

impl IndexWriter {
    /// Create a brand-new v8 scratch index without overwriting an existing
    /// path. This is the safe primitive used by scratch sync and copy migration.
    pub fn create_v8(path: impl AsRef<Path>) -> IndexResult<Self> {
        let path = path.as_ref().to_path_buf();
        if path.exists() {
            return Err(IndexError::ScratchExists(path));
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Reserve the exact path first so a concurrent creator cannot be
        // silently overwritten by SQLite's CREATE flag.
        let mut reserve = OpenOptions::new();
        reserve.write(true).create_new(true);
        // Index projections may contain private conversation history.  Do not
        // rely on the caller's umask for a newly-created active/scratch DB.
        #[cfg(unix)]
        reserve.mode(0o600);
        reserve.open(&path)?;
        let result = (|| {
            let mut connection = Connection::open_with_flags(
                &path,
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            configure_writer(&connection)?;
            initialize_v8(&mut connection)?;
            Ok(Self {
                connection,
                path: path.clone(),
            })
        })();

        if result.is_err() {
            // The file was created by this call and has never been published;
            // removing it is safe and keeps retry semantics deterministic.
            let _ = fs::remove_file(&path);
            let _ = fs::remove_file(sqlite_sidecar_path(&path, "-wal"));
            let _ = fs::remove_file(sqlite_sidecar_path(&path, "-shm"));
        }
        result
    }

    pub fn open_v8(path: impl AsRef<Path>) -> IndexResult<Self> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Err(IndexError::NotFound(path));
        }
        let connection = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))?;
        let layout = detect_layout(&connection)?;
        if layout != IndexLayout::V8 {
            return Err(IndexError::InvalidOperation(
                "v7 indexes are read-only until explicit copy migration".to_owned(),
            ));
        }
        // Reject incompatible generations before journal/WAL configuration can
        // touch the database or its sidecars.
        validate_v8_writer_metadata(&connection)?;
        configure_writer(&connection)?;
        Ok(Self { connection, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn begin(&mut self) -> IndexResult<IndexTransaction<'_>> {
        Ok(IndexTransaction {
            transaction: self.connection.transaction()?,
        })
    }

    pub fn set_migration_receipt(&mut self, receipt: &str) -> IndexResult<()> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO meta(key, value) VALUES ('migration_receipt', ?) \
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [receipt],
        )?;
        transaction.execute(
            "INSERT INTO meta(key, value) \
             VALUES ('upgraded_at', strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) \
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [],
        )?;
        transaction.commit()?;
        Ok(())
    }
}

/// A concrete transaction used by the future sync state machine. Session,
/// document, FTS, source-file, and coverage mutations can be committed as one
/// SQLite unit without exposing the connection or physical rowids upstream.
pub struct IndexTransaction<'connection> {
    transaction: Transaction<'connection>,
}

impl IndexTransaction<'_> {
    pub fn replace_session(
        &mut self,
        session: &SessionWrite,
        messages: &[MessageWrite],
    ) -> IndexResult<i64> {
        validate_message_sequence(messages, 0)?;
        let session_id = upsert_session(&self.transaction, session)?;
        delete_documents(&self.transaction, session_id, None)?;
        insert_profile(&self.transaction, session_id, &session.profile())?;
        for message in messages {
            insert_message(&self.transaction, session_id, message)?;
        }
        update_counts(&self.transaction, session_id)?;
        Ok(session_id)
    }

    /// Update the stable session metadata/profile and append a proven
    /// contiguous message tail. `expected_next_seq` makes stale append plans
    /// fail rather than duplicate or skip evidence.
    pub fn append_session(
        &mut self,
        session: &SessionWrite,
        expected_next_seq: i64,
        messages: &[MessageWrite],
    ) -> IndexResult<i64> {
        let session_id = require_session_id(&self.transaction, &session.identity)?;
        assert_next_seq(&self.transaction, session_id, expected_next_seq)?;
        validate_message_sequence(messages, expected_next_seq)?;
        let updated_id = upsert_session(&self.transaction, session)?;
        debug_assert_eq!(updated_id, session_id);
        replace_profile_by_id(&self.transaction, session_id, &session.profile())?;
        for message in messages {
            insert_message(&self.transaction, session_id, message)?;
        }
        update_counts(&self.transaction, session_id)?;
        Ok(session_id)
    }

    pub fn replace_profile(
        &mut self,
        identity: &SessionIdentity,
        profile: &SessionProfileWrite,
    ) -> IndexResult<()> {
        validate_profile(profile)?;
        let session_id = require_session_id(&self.transaction, identity)?;
        self.transaction.execute(
            "UPDATE session_rows SET title=?, summary_text=?, compact_text=?, \
             reasoning_summary_text=?, updated_at=CURRENT_TIMESTAMP WHERE id=?",
            params![
                profile.title_text,
                profile.summary_text,
                profile.compact_text,
                profile.reasoning_text,
                session_id,
            ],
        )?;
        replace_profile_by_id(&self.transaction, session_id, profile)?;
        update_counts(&self.transaction, session_id)?;
        Ok(())
    }

    pub fn replace_messages(
        &mut self,
        identity: &SessionIdentity,
        messages: &[MessageWrite],
    ) -> IndexResult<()> {
        validate_message_sequence(messages, 0)?;
        let session_id = require_session_id(&self.transaction, identity)?;
        delete_documents(&self.transaction, session_id, Some(DocumentKind::Message))?;
        for message in messages {
            insert_message(&self.transaction, session_id, message)?;
        }
        update_counts(&self.transaction, session_id)?;
        Ok(())
    }

    pub fn append_messages(
        &mut self,
        identity: &SessionIdentity,
        expected_next_seq: i64,
        messages: &[MessageWrite],
    ) -> IndexResult<()> {
        let session_id = require_session_id(&self.transaction, identity)?;
        assert_next_seq(&self.transaction, session_id, expected_next_seq)?;
        validate_message_sequence(messages, expected_next_seq)?;
        for message in messages {
            insert_message(&self.transaction, session_id, message)?;
        }
        update_counts(&self.transaction, session_id)?;
        Ok(())
    }

    pub fn upsert_source_file(&mut self, state: &SourceFileState) -> IndexResult<()> {
        validate_source_file(state)?;
        let session_id = match &state.session {
            Some(identity) => Some(require_session_id(&self.transaction, identity)?),
            None => None,
        };
        self.transaction.execute(
            "INSERT INTO source_files( \
               source_id, file_path, session_id, source_root, source_generation, \
               mtime_ms, mtime_ns, size, indexed_bytes, head_digest, boundary_digest, next_seq, \
               reducer_checkpoint, cwd, path_date, extra_fingerprint, projection_epoch, \
               analyzer_epoch, coverage_epoch, updated_at \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP) \
             ON CONFLICT(source_id, file_path) DO UPDATE SET \
               session_id=excluded.session_id, source_root=excluded.source_root, \
               source_generation=excluded.source_generation, mtime_ms=excluded.mtime_ms, \
               mtime_ns=excluded.mtime_ns, \
               size=excluded.size, indexed_bytes=excluded.indexed_bytes, \
               head_digest=excluded.head_digest, boundary_digest=excluded.boundary_digest, \
               next_seq=excluded.next_seq, reducer_checkpoint=excluded.reducer_checkpoint, \
               cwd=excluded.cwd, path_date=excluded.path_date, \
               extra_fingerprint=excluded.extra_fingerprint, \
               projection_epoch=excluded.projection_epoch, analyzer_epoch=excluded.analyzer_epoch, \
               coverage_epoch=excluded.coverage_epoch, updated_at=CURRENT_TIMESTAMP",
            params![
                state.source_id.as_str(),
                state.file_path,
                session_id,
                state.source_root,
                state.source_generation,
                state.mtime_ms,
                state.mtime_ns,
                to_i64(state.size, "source file size")?,
                to_i64(state.indexed_bytes, "indexed bytes")?,
                state.head_digest,
                state.boundary_digest,
                state.next_seq,
                state.reducer_checkpoint,
                state.cwd,
                state.path_date,
                state.extra_fingerprint,
                state.projection_epoch,
                state.analyzer_epoch,
                state.coverage_epoch,
            ],
        )?;
        Ok(())
    }

    /// Refresh raw-file stat/cursor state without replacing searchable
    /// documents. The caller must first prove the raw bytes are unchanged.
    pub fn refresh_source_file(
        &mut self,
        state: &SourceFileState,
        raw_file_mtime: i64,
    ) -> IndexResult<()> {
        self.upsert_source_file(state)?;
        if let Some(identity) = &state.session {
            let session_id = require_session_id(&self.transaction, identity)?;
            self.transaction.execute(
                "UPDATE session_rows SET raw_file_mtime=?, raw_file_size=?, \
                 updated_at=CURRENT_TIMESTAMP WHERE id=?",
                params![
                    raw_file_mtime,
                    to_i64(state.size, "raw file size")?,
                    session_id
                ],
            )?;
        }
        Ok(())
    }

    pub fn delete_source_file(&mut self, source: SourceId, file_path: &str) -> IndexResult<bool> {
        let changes = self.transaction.execute(
            "DELETE FROM source_files WHERE source_id=? AND file_path=?",
            params![source.as_str(), file_path],
        )?;
        Ok(changes > 0)
    }

    pub fn delete_session_by_file(
        &mut self,
        source: SourceId,
        file_path: &str,
    ) -> IndexResult<bool> {
        let session_id = self
            .transaction
            .query_row(
                "SELECT id FROM session_rows WHERE source_id=? AND file_path=? LIMIT 1",
                params![source.as_str(), file_path],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        match session_id {
            Some(session_id) => {
                delete_session_id(&self.transaction, session_id)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Remove both source cursor and stored projection for a parser-filtered
    /// raw file. The return value says whether either row existed.
    pub fn delete_filtered_file(&mut self, source: SourceId, file_path: &str) -> IndexResult<bool> {
        let removed_session = self.delete_session_by_file(source, file_path)?;
        let removed_source_file = self.delete_source_file(source, file_path)?;
        Ok(removed_session || removed_source_file)
    }

    pub fn selector_counts(&self, selector: &Selector) -> IndexResult<SelectorCounts> {
        let (conditions, values) = selector_filter(selector, "s")?;
        let predicate = conditions.join(" AND ");
        let (session_count, message_document_count): (i64, i64) = self.transaction.query_row(
            &format!(
                "SELECT COUNT(*), COALESCE(SUM(message_count), 0) \
                     FROM session_rows s WHERE {predicate}"
            ),
            params_from_iter(values.clone()),
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let document_count: i64 = self.transaction.query_row(
            &format!(
                "SELECT COUNT(*) FROM documents d JOIN session_rows s ON s.id=d.session_id \
                 WHERE {predicate}"
            ),
            params_from_iter(values),
            |row| row.get(0),
        )?;
        Ok(SelectorCounts {
            session_count: non_negative_i64(session_count, "selector session count")?,
            message_document_count: non_negative_i64(
                message_document_count,
                "selector message count",
            )?,
            document_count: non_negative_i64(document_count, "selector document count")?,
        })
    }

    pub fn prune(
        &mut self,
        selector: &Selector,
        retained_hot_paths: &HashSet<String>,
        retained_cold_native_ids: &HashSet<String>,
    ) -> IndexResult<PruneOutcome> {
        let (conditions, values) = selector_filter(selector, "s")?;
        let sql = format!(
            "SELECT s.id, s.file_path, s.native_session_id FROM session_rows s WHERE {}",
            conditions.join(" AND ")
        );
        let mut statement = self.transaction.prepare(&sql)?;
        let rows = statement
            .query_map(params_from_iter(values), |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);

        let cold = retained_cold_native_ids
            .iter()
            .map(|value| value.to_lowercase())
            .collect::<HashSet<_>>();
        let mut outcome = PruneOutcome::default();
        for (session_id, file_path, native_session_id) in rows {
            if retained_hot_paths.contains(&file_path) {
                continue;
            }
            if cold.contains(&native_session_id.to_lowercase()) {
                outcome.retained_cold += 1;
                continue;
            }
            delete_session_id(&self.transaction, session_id)?;
            outcome.removed += 1;
        }
        Ok(outcome)
    }

    pub fn upsert_cold_root(
        &mut self,
        source: SourceId,
        root: &str,
        added_at: Option<&str>,
    ) -> IndexResult<()> {
        if root.trim().is_empty() {
            return Err(IndexError::InvalidOperation(
                "cold root must be non-empty".to_owned(),
            ));
        }
        match added_at {
            Some(added_at) => self.transaction.execute(
                "INSERT INTO cold_roots(source_id, root, added_at) VALUES (?, ?, ?) \
                 ON CONFLICT(source_id, root) DO UPDATE SET added_at=excluded.added_at",
                params![source.as_str(), root, added_at],
            )?,
            None => self.transaction.execute(
                "INSERT INTO cold_roots(source_id, root, added_at) \
                 VALUES (?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) \
                 ON CONFLICT(source_id, root) DO NOTHING",
                params![source.as_str(), root],
            )?,
        };
        Ok(())
    }

    pub fn remove_cold_root(&mut self, source: SourceId, root: &str) -> IndexResult<bool> {
        let changes = self.transaction.execute(
            "DELETE FROM cold_roots WHERE source_id=? AND root=?",
            params![source.as_str(), root],
        )?;
        Ok(changes > 0)
    }

    pub fn replace_coverage(&mut self, coverage: &CoverageWrite) -> IndexResult<()> {
        validate_coverage(coverage)?;
        let selector_key = coverage.selector.storage_key();
        let selector_json = serde_json::to_string(&coverage.selector)?;
        let (cwd, from_date, to_date) = selector_parts(&coverage.selector);
        let completed_at = coverage.completed_at.clone().unwrap_or_else(|| {
            // SQLite owns canonical write timestamps; this marker is replaced
            // by CURRENT_TIMESTAMP in the dedicated branch below.
            String::new()
        });
        if completed_at.is_empty() {
            self.transaction.execute(
                &coverage_upsert_sql("CURRENT_TIMESTAMP"),
                params![
                    coverage.selector.source().as_str(),
                    selector_key,
                    selector_json,
                    selector_kind_text(coverage.selector.kind()),
                    coverage.selector.root(),
                    cwd,
                    from_date,
                    to_date,
                    coverage.source_fingerprint,
                    coverage.source_file_set_fingerprint,
                    to_i64(coverage.source_file_count, "coverage file count")?,
                    to_i64(coverage.indexed_session_count, "coverage session count")?,
                    to_i64(coverage.indexed_document_count, "coverage document count")?,
                    coverage.source_generation,
                    coverage.index_version,
                    coverage.projection_epoch,
                    coverage.analyzer_epoch,
                    coverage.coverage_epoch,
                ],
            )?;
        } else {
            self.transaction.execute(
                &coverage_upsert_sql("?"),
                params![
                    coverage.selector.source().as_str(),
                    selector_key,
                    selector_json,
                    selector_kind_text(coverage.selector.kind()),
                    coverage.selector.root(),
                    cwd,
                    from_date,
                    to_date,
                    coverage.source_fingerprint,
                    coverage.source_file_set_fingerprint,
                    to_i64(coverage.source_file_count, "coverage file count")?,
                    to_i64(coverage.indexed_session_count, "coverage session count")?,
                    to_i64(coverage.indexed_document_count, "coverage document count")?,
                    coverage.source_generation,
                    completed_at,
                    coverage.index_version,
                    coverage.projection_epoch,
                    coverage.analyzer_epoch,
                    coverage.coverage_epoch,
                ],
            )?;
        }
        Ok(())
    }

    /// Invalidate every stored proof for `requested`'s source/root.
    ///
    /// A non-complete sync can change projection rows inside either a narrower
    /// or broader stored proof. Selector implication in only one direction is
    /// therefore insufficient: a partial `all(root)` sync can also invalidate
    /// an existing `cwd(root, /a)` proof. Conservatively dropping every proof
    /// for the same source/root prevents stale completeness claims.
    pub fn invalidate_covering_coverage(&mut self, requested: &Selector) -> IndexResult<u64> {
        self.invalidate_coverage_for_source_root(requested.source(), requested.root())
    }

    /// Invalidate every coverage proof for one raw source/root namespace.
    pub fn invalidate_coverage_for_source_root(
        &mut self,
        source: SourceId,
        root: &str,
    ) -> IndexResult<u64> {
        let changes = self.transaction.execute(
            "DELETE FROM coverage WHERE source_id=? AND root=?",
            params![source.as_str(), root],
        )?;
        u64::try_from(changes)
            .map_err(|_| IndexError::InvalidData("coverage delete count exceeds u64".to_owned()))
    }

    /// Copy a stored projection without consulting raw source. Callers doing
    /// a generation migration must explicitly translate the bundle to the
    /// current index version/epochs first; this writer never normalizes them.
    pub fn copy_session_bundle(&mut self, bundle: &SessionBundle) -> IndexResult<i64> {
        let profile = bundle
            .documents
            .iter()
            .find(|document| document.kind == DocumentKind::SessionProfile)
            .ok_or_else(|| {
                IndexError::InvalidOperation(
                    "session bundle must contain exactly one session_profile".to_owned(),
                )
            })?;
        if bundle
            .documents
            .iter()
            .filter(|document| document.kind == DocumentKind::SessionProfile)
            .count()
            != 1
        {
            return Err(IndexError::InvalidOperation(
                "session bundle must contain exactly one session_profile".to_owned(),
            ));
        }
        let profile = profile_write(profile)?;
        let mut messages = bundle
            .documents
            .iter()
            .filter(|document| document.kind == DocumentKind::Message)
            .map(message_write)
            .collect::<IndexResult<Vec<_>>>()?;
        messages.sort_by_key(|message| message.seq);
        validate_message_sequence(&messages, 0)?;

        let session = bundle.session.as_write();
        let session_id = upsert_session(&self.transaction, &session)?;
        delete_documents(&self.transaction, session_id, None)?;
        insert_profile(&self.transaction, session_id, &profile)?;
        for message in &messages {
            insert_message(&self.transaction, session_id, message)?;
        }
        update_counts(&self.transaction, session_id)?;
        for source_file in &bundle.source_files {
            self.upsert_source_file(source_file)?;
        }
        Ok(session_id)
    }

    /// Validate logical relationships immediately before publication. A failed
    /// check drops and rolls back the SQLite transaction.
    pub fn commit(self) -> IndexResult<CommitReceipt> {
        let report = inspect_v8_invariants(&self.transaction)?;
        if !report.is_valid() {
            return Err(IndexError::Invariant(report.violations.join("; ")));
        }
        let committed_at: String = self.transaction.query_row(
            "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            [],
            |row| row.get(0),
        )?;
        self.transaction.commit()?;
        Ok(CommitReceipt {
            committed_at,
            invariants: report,
        })
    }

    pub fn rollback(self) -> IndexResult<()> {
        self.transaction.rollback()?;
        Ok(())
    }
}

fn configure_writer(connection: &Connection) -> IndexResult<()> {
    connection.busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    // Keep the published index as a single-file rollback-journal database.
    // A read-only WAL connection may create `-wal`/`-shm` files (and cannot be
    // opened from read-only media when those files are absent), which violates
    // the command contract for status/find/read/list/stats. Writers are already
    // serialized by the app-level lock, so prefer a quiescent single-file index
    // over WAL's reader/writer concurrency here.
    let journal_mode: String =
        connection.query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))?;
    if !journal_mode.eq_ignore_ascii_case("delete") {
        return Err(IndexError::InvalidOperation(format!(
            "SQLite refused journal_mode=DELETE and remained {journal_mode:?}"
        )));
    }
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "temp_store", "MEMORY")?;
    Ok(())
}

fn upsert_session(connection: &Connection, session: &SessionWrite) -> IndexResult<i64> {
    validate_session(session)?;
    if let Some(owner) = connection
        .query_row(
            "SELECT session_key FROM session_rows \
             WHERE source_id=? AND file_path=? AND native_session_id<>? LIMIT 1",
            params![
                session.identity.source_id.as_str(),
                session.file_path,
                session.identity.native_session_id,
            ],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        return Err(IndexError::InvalidOperation(format!(
            "source file {:?} already belongs to session {owner:?}",
            session.file_path
        )));
    }

    connection.execute(
        "INSERT INTO session_rows( \
           source_id, native_session_id, session_key, session_uuid, file_path, source_root, \
           title, summary_text, compact_text, reasoning_summary_text, cwd, model, \
           started_at, ended_at, path_date, raw_file_mtime, raw_file_size, index_version, updated_at \
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP) \
         ON CONFLICT(source_id, native_session_id) DO UPDATE SET \
           session_key=excluded.session_key, session_uuid=excluded.session_uuid, \
           file_path=excluded.file_path, source_root=excluded.source_root, title=excluded.title, \
           summary_text=excluded.summary_text, compact_text=excluded.compact_text, \
           reasoning_summary_text=excluded.reasoning_summary_text, cwd=excluded.cwd, model=excluded.model, \
           started_at=excluded.started_at, ended_at=excluded.ended_at, path_date=excluded.path_date, \
           raw_file_mtime=excluded.raw_file_mtime, raw_file_size=excluded.raw_file_size, \
           index_version=excluded.index_version, updated_at=CURRENT_TIMESTAMP",
        params![
            session.identity.source_id.as_str(),
            session.identity.native_session_id,
            session.identity.session_key,
            session.session_uuid,
            session.file_path,
            session.source_root,
            session.title,
            session.summary_text,
            session.compact_text,
            session.reasoning_summary_text,
            session.cwd,
            session.model,
            session.started_at,
            session.ended_at,
            session.path_date,
            session.raw_file_mtime,
            to_i64(session.raw_file_size, "raw file size")?,
            session.index_version,
        ],
    )?;
    require_session_id(connection, &session.identity)
}

fn require_session_id(connection: &Connection, identity: &SessionIdentity) -> IndexResult<i64> {
    connection
        .query_row(
            "SELECT id FROM session_rows WHERE source_id=? AND native_session_id=? LIMIT 1",
            params![identity.source_id.as_str(), identity.native_session_id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| IndexError::SessionNotFound(identity.session_key.clone()))
}

fn replace_profile_by_id(
    connection: &Connection,
    session_id: i64,
    profile: &SessionProfileWrite,
) -> IndexResult<()> {
    delete_documents(connection, session_id, Some(DocumentKind::SessionProfile))?;
    insert_profile(connection, session_id, profile)
}

fn insert_profile(
    connection: &Connection,
    session_id: i64,
    profile: &SessionProfileWrite,
) -> IndexResult<()> {
    validate_profile(profile)?;
    connection.execute(
        "INSERT INTO documents( \
           session_id, kind, seq, role, timestamp, source_kind, body_text, title_text, \
           summary_text, compact_text, reasoning_text, raw_start, raw_end, projection_epoch \
         ) VALUES (?, 'session_profile', NULL, NULL, NULL, NULL, '', ?, ?, ?, ?, ?, ?, ?)",
        params![
            session_id,
            profile.title_text,
            profile.summary_text,
            profile.compact_text,
            profile.reasoning_text,
            optional_u64(profile.raw_start, "profile raw_start")?,
            optional_u64(profile.raw_end, "profile raw_end")?,
            profile.projection_epoch,
        ],
    )?;
    let document_id = connection.last_insert_rowid();
    insert_fts(
        connection,
        document_id,
        "",
        &profile.title_text,
        &profile.summary_text,
        &profile.compact_text,
        &profile.reasoning_text,
    )
}

fn validate_profile(profile: &SessionProfileWrite) -> IndexResult<()> {
    validate_offsets(profile.raw_start, profile.raw_end)?;
    if profile.projection_epoch != PROJECTION_EPOCH {
        return Err(IndexError::InvalidOperation(format!(
            "profile projection_epoch must be {PROJECTION_EPOCH}, got {}",
            profile.projection_epoch
        )));
    }
    Ok(())
}

fn insert_message(
    connection: &Connection,
    session_id: i64,
    message: &MessageWrite,
) -> IndexResult<()> {
    validate_offsets(message.raw_start, message.raw_end)?;
    let role = role_text(message.role);
    connection.execute(
        "INSERT INTO documents( \
           session_id, kind, seq, role, timestamp, source_kind, body_text, title_text, \
           summary_text, compact_text, reasoning_text, raw_start, raw_end, projection_epoch \
         ) VALUES (?, 'message', ?, ?, ?, ?, ?, '', '', '', '', ?, ?, ?)",
        params![
            session_id,
            message.seq,
            role,
            message.timestamp,
            message.source_kind,
            message.body_text,
            optional_u64(message.raw_start, "message raw_start")?,
            optional_u64(message.raw_end, "message raw_end")?,
            message.projection_epoch,
        ],
    )?;
    let document_id = connection.last_insert_rowid();
    insert_fts(connection, document_id, &message.body_text, "", "", "", "")
}

fn insert_fts(
    connection: &Connection,
    document_id: i64,
    body: &str,
    title: &str,
    summary: &str,
    compact: &str,
    reasoning: &str,
) -> IndexResult<()> {
    connection.execute(
        "INSERT INTO documents_fts( \
           rowid, body_text, title_text, summary_text, compact_text, reasoning_text \
         ) VALUES (?, ?, ?, ?, ?, ?)",
        params![
            document_id,
            tokenized_text(body),
            tokenized_text(title),
            tokenized_text(summary),
            tokenized_text(compact),
            tokenized_text(reasoning),
        ],
    )?;
    Ok(())
}

fn delete_documents(
    connection: &Connection,
    session_id: i64,
    kind: Option<DocumentKind>,
) -> IndexResult<()> {
    let (sql, parameters): (&str, Vec<rusqlite::types::Value>) = match kind {
        Some(kind) => (
            "SELECT id FROM documents WHERE session_id=? AND kind=?",
            vec![session_id.into(), sql_text(kind.as_str())],
        ),
        None => (
            "SELECT id FROM documents WHERE session_id=?",
            vec![session_id.into()],
        ),
    };
    let mut statement = connection.prepare(sql)?;
    let ids = statement
        .query_map(rusqlite::params_from_iter(parameters), |row| {
            row.get::<_, i64>(0)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for id in ids {
        connection.execute("DELETE FROM documents_fts WHERE rowid=?", [id])?;
        connection.execute("DELETE FROM documents WHERE id=?", [id])?;
    }
    Ok(())
}

fn update_counts(connection: &Connection, session_id: i64) -> IndexResult<()> {
    connection.execute(
        "UPDATE session_rows SET \
           message_count=(SELECT COUNT(*) FROM documents WHERE session_id=? AND kind='message'), \
           document_count=(SELECT COUNT(*) FROM documents WHERE session_id=?), \
           updated_at=CURRENT_TIMESTAMP \
         WHERE id=?",
        params![session_id, session_id, session_id],
    )?;
    Ok(())
}

fn delete_session_id(connection: &Connection, session_id: i64) -> IndexResult<()> {
    delete_documents(connection, session_id, None)?;
    connection.execute("DELETE FROM source_files WHERE session_id=?", [session_id])?;
    connection.execute("DELETE FROM session_rows WHERE id=?", [session_id])?;
    Ok(())
}

fn assert_next_seq(connection: &Connection, session_id: i64, expected: i64) -> IndexResult<()> {
    if expected < 0 {
        return Err(IndexError::InvalidOperation(
            "expected_next_seq must be non-negative".to_owned(),
        ));
    }
    let actual: i64 = connection.query_row(
        "SELECT COALESCE(MAX(seq) + 1, 0) FROM documents \
         WHERE session_id=? AND kind='message'",
        [session_id],
        |row| row.get(0),
    )?;
    if actual != expected {
        return Err(IndexError::InvalidOperation(format!(
            "append cursor mismatch: expected next_seq {expected}, stored next_seq {actual}"
        )));
    }
    Ok(())
}

fn validate_message_sequence(messages: &[MessageWrite], first_seq: i64) -> IndexResult<()> {
    if first_seq < 0 {
        return Err(IndexError::InvalidOperation(
            "message sequence must start at a non-negative value".to_owned(),
        ));
    }
    for (offset, message) in messages.iter().enumerate() {
        let expected = first_seq
            .checked_add(i64::try_from(offset).map_err(|_| {
                IndexError::InvalidOperation("message batch is too large".to_owned())
            })?)
            .ok_or_else(|| IndexError::InvalidOperation("message sequence overflow".to_owned()))?;
        if message.seq != expected {
            return Err(IndexError::InvalidOperation(format!(
                "message seq must be contiguous: expected {expected}, got {}",
                message.seq
            )));
        }
        validate_offsets(message.raw_start, message.raw_end)?;
        if message.projection_epoch != PROJECTION_EPOCH {
            return Err(IndexError::InvalidOperation(format!(
                "message projection_epoch must be {PROJECTION_EPOCH}, got {}",
                message.projection_epoch
            )));
        }
    }
    Ok(())
}

fn validate_session(session: &SessionWrite) -> IndexResult<()> {
    if session.identity.native_session_id.is_empty()
        || session.identity.session_key.is_empty()
        || session.session_uuid.is_empty()
        || session.file_path.is_empty()
        || session.started_at.is_empty()
        || session.ended_at.is_empty()
    {
        return Err(IndexError::InvalidOperation(
            "session identity, path, UUID, and timestamps must be non-empty".to_owned(),
        ));
    }
    let canonical_key = format!(
        "{}:{}",
        session.identity.source_id, session.identity.native_session_id
    );
    if session.identity.session_key != canonical_key {
        return Err(IndexError::InvalidOperation(format!(
            "session_key must be canonical: expected {canonical_key:?}"
        )));
    }
    if session.index_version != INDEX_VERSION {
        return Err(IndexError::InvalidOperation(format!(
            "session index_version must be {INDEX_VERSION:?}, got {:?}",
            session.index_version
        )));
    }
    Ok(())
}

fn validate_source_file(state: &SourceFileState) -> IndexResult<()> {
    if !state.mtime_ms.is_finite() || state.mtime_ms < 0.0 {
        return Err(IndexError::InvalidOperation(
            "source file mtime_ms must be finite and non-negative".to_owned(),
        ));
    }
    if state.mtime_ns.is_some_and(|mtime_ns| mtime_ns < 0) {
        return Err(IndexError::InvalidOperation(
            "source file mtime_ns must be non-negative when present".to_owned(),
        ));
    }
    if state.indexed_bytes > state.size {
        return Err(IndexError::InvalidOperation(
            "source file indexed_bytes cannot exceed size".to_owned(),
        ));
    }
    if state.next_seq < 0 {
        return Err(IndexError::InvalidOperation(
            "source file next_seq must be non-negative".to_owned(),
        ));
    }
    if state.indexed_bytes > 0 && (state.head_digest.is_empty() || state.boundary_digest.is_empty())
    {
        return Err(IndexError::InvalidOperation(
            "an established append cursor requires head and boundary digests".to_owned(),
        ));
    }
    if state.projection_epoch != PROJECTION_EPOCH
        || state.analyzer_epoch != ANALYZER_EPOCH
        || state.coverage_epoch != COVERAGE_EPOCH
    {
        return Err(IndexError::InvalidOperation(format!(
            "source file epochs must be projection={PROJECTION_EPOCH}, analyzer={ANALYZER_EPOCH}, coverage={COVERAGE_EPOCH}; got projection={}, analyzer={}, coverage={}",
            state.projection_epoch, state.analyzer_epoch, state.coverage_epoch
        )));
    }
    if state.source_id
        != state
            .session
            .as_ref()
            .map_or(state.source_id, |value| value.source_id)
    {
        return Err(IndexError::InvalidOperation(
            "source file and linked session source must match".to_owned(),
        ));
    }
    Ok(())
}

fn validate_coverage(coverage: &CoverageWrite) -> IndexResult<()> {
    if coverage.projection_epoch != PROJECTION_EPOCH
        || coverage.analyzer_epoch != ANALYZER_EPOCH
        || coverage.coverage_epoch != COVERAGE_EPOCH
    {
        return Err(IndexError::InvalidOperation(format!(
            "coverage epochs must be projection={PROJECTION_EPOCH}, analyzer={ANALYZER_EPOCH}, coverage={COVERAGE_EPOCH}; got projection={}, analyzer={}, coverage={}",
            coverage.projection_epoch, coverage.analyzer_epoch, coverage.coverage_epoch
        )));
    }
    if coverage.index_version != INDEX_VERSION {
        return Err(IndexError::InvalidOperation(format!(
            "coverage index_version must be {INDEX_VERSION:?}, got {:?}",
            coverage.index_version
        )));
    }
    Ok(())
}

fn validate_offsets(start: Option<u64>, end: Option<u64>) -> IndexResult<()> {
    match (start, end) {
        (None, None) => Ok(()),
        (Some(start), Some(end)) if start <= end => Ok(()),
        _ => Err(IndexError::InvalidOperation(
            "raw_start/raw_end must both be null or form an ordered pair".to_owned(),
        )),
    }
}

fn profile_write(document: &StoredDocument) -> IndexResult<SessionProfileWrite> {
    if document.kind != DocumentKind::SessionProfile {
        return Err(IndexError::InvalidOperation(
            "expected session_profile document".to_owned(),
        ));
    }
    if document.projection_epoch != PROJECTION_EPOCH {
        return Err(IndexError::InvalidOperation(format!(
            "stored profile projection_epoch must be {PROJECTION_EPOCH}, got {}",
            document.projection_epoch
        )));
    }
    Ok(SessionProfileWrite {
        title_text: document.title_text.clone(),
        summary_text: document.summary_text.clone(),
        compact_text: document.compact_text.clone(),
        reasoning_text: document.reasoning_text.clone(),
        raw_start: document.raw_start,
        raw_end: document.raw_end,
        projection_epoch: document.projection_epoch,
    })
}

fn message_write(document: &StoredDocument) -> IndexResult<MessageWrite> {
    if document.kind != DocumentKind::Message {
        return Err(IndexError::InvalidOperation(
            "expected message document".to_owned(),
        ));
    }
    if document.projection_epoch != PROJECTION_EPOCH {
        return Err(IndexError::InvalidOperation(format!(
            "stored message projection_epoch must be {PROJECTION_EPOCH}, got {}",
            document.projection_epoch
        )));
    }
    Ok(MessageWrite {
        seq: document
            .seq
            .ok_or_else(|| IndexError::InvalidData("message seq is null".to_owned()))?,
        role: document
            .role
            .ok_or_else(|| IndexError::InvalidData("message role is null".to_owned()))?,
        timestamp: document.timestamp.clone().unwrap_or_default(),
        source_kind: document
            .source_kind
            .clone()
            .unwrap_or_else(|| "event_msg".to_owned()),
        body_text: document.body_text.clone(),
        raw_start: document.raw_start,
        raw_end: document.raw_end,
        projection_epoch: document.projection_epoch,
    })
}

fn selector_parts(selector: &Selector) -> (Option<&str>, Option<&str>, Option<&str>) {
    match selector {
        Selector::All { .. } => (None, None, None),
        Selector::DateRange {
            from_date, to_date, ..
        } => (None, Some(from_date), Some(to_date)),
        Selector::Cwd { cwd, .. } => (Some(cwd), None, None),
        Selector::CwdDateRange {
            cwd,
            from_date,
            to_date,
            ..
        } => (Some(cwd), Some(from_date), Some(to_date)),
    }
}

fn selector_kind_text(kind: SelectorKind) -> &'static str {
    match kind {
        SelectorKind::All => "all",
        SelectorKind::DateRange => "date_range",
        SelectorKind::Cwd => "cwd",
        SelectorKind::CwdDateRange => "cwd_date_range",
    }
}

fn coverage_upsert_sql(completed_at_expression: &str) -> String {
    format!(
        "INSERT INTO coverage( \
           source_id, selector_key, selector_json, selector_kind, root, cwd, from_date, to_date, \
           source_fingerprint, source_file_set_fingerprint, source_file_count, indexed_session_count, \
           indexed_document_count, source_generation, completed_at, index_version, projection_epoch, \
           analyzer_epoch, coverage_epoch \
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, {completed_at_expression}, ?, ?, ?, ?) \
         ON CONFLICT(selector_key) DO UPDATE SET \
           source_id=excluded.source_id, selector_json=excluded.selector_json, \
           selector_kind=excluded.selector_kind, root=excluded.root, cwd=excluded.cwd, \
           from_date=excluded.from_date, to_date=excluded.to_date, \
           source_fingerprint=excluded.source_fingerprint, \
           source_file_set_fingerprint=excluded.source_file_set_fingerprint, \
           source_file_count=excluded.source_file_count, indexed_session_count=excluded.indexed_session_count, \
           indexed_document_count=excluded.indexed_document_count, source_generation=excluded.source_generation, \
           completed_at=excluded.completed_at, index_version=excluded.index_version, \
           projection_epoch=excluded.projection_epoch, analyzer_epoch=excluded.analyzer_epoch, \
           coverage_epoch=excluded.coverage_epoch"
    )
}

fn selector_filter(selector: &Selector, alias: &str) -> IndexResult<(Vec<String>, Vec<SqlValue>)> {
    if !alias
        .chars()
        .all(|value| value.is_ascii_alphanumeric() || value == '_')
    {
        return Err(IndexError::InvalidOperation("invalid SQL alias".to_owned()));
    }
    let mut conditions = vec![
        format!("{alias}.source_id=?"),
        format!("({alias}.file_path=? OR {alias}.file_path LIKE ? ESCAPE '\\')"),
    ];
    let mut values = vec![
        sql_text(selector.source().as_str()),
        sql_text(selector.root()),
        SqlValue::from(descendant_like_pattern(selector.root())),
    ];
    match selector {
        Selector::All { .. } => {}
        Selector::DateRange {
            from_date, to_date, ..
        } => {
            conditions.push(format!("{alias}.path_date>=?"));
            conditions.push(format!("{alias}.path_date<=?"));
            values.push(sql_text(from_date));
            values.push(sql_text(to_date));
        }
        Selector::Cwd { cwd, .. } => {
            conditions.push(format!("{alias}.cwd=?"));
            values.push(sql_text(cwd));
        }
        Selector::CwdDateRange {
            cwd,
            from_date,
            to_date,
            ..
        } => {
            conditions.push(format!("{alias}.cwd=?"));
            conditions.push(format!("{alias}.path_date>=?"));
            conditions.push(format!("{alias}.path_date<=?"));
            values.push(sql_text(cwd));
            values.push(sql_text(from_date));
            values.push(sql_text(to_date));
        }
    }
    Ok((conditions, values))
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn descendant_like_pattern(root: &str) -> String {
    let mut prefix = root.to_owned();
    if !prefix.ends_with(std::path::MAIN_SEPARATOR) {
        prefix.push(std::path::MAIN_SEPARATOR);
    }
    format!("{}%", escape_like(&prefix))
}

fn sql_text(value: &str) -> SqlValue {
    SqlValue::Text(value.to_owned())
}

fn role_text(role: MessageRole) -> &'static str {
    match role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
    }
}

fn optional_u64(value: Option<u64>, field: &str) -> IndexResult<Option<i64>> {
    value.map(|value| to_i64(value, field)).transpose()
}

fn to_i64(value: u64, field: &str) -> IndexResult<i64> {
    i64::try_from(value)
        .map_err(|_| IndexError::InvalidOperation(format!("{field} exceeds SQLite INTEGER range")))
}

fn non_negative_i64(value: i64, field: &str) -> IndexResult<u64> {
    u64::try_from(value)
        .map_err(|_| IndexError::InvalidData(format!("{field} is negative: {value}")))
}

fn sqlite_sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value: OsString = path.as_os_str().to_owned();
    value.push(suffix);
    PathBuf::from(value)
}
