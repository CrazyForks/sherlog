use std::collections::HashSet;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use rusqlite::Connection;
use tempfile::TempDir;

use crate::config::INDEX_VERSION;
use crate::identity::{SessionIdentity, SessionRef, SourceId};
use crate::model::{MessageRole, SessionListQuery, SessionListSort};
use crate::selector::Selector;

use super::*;

fn db_path(directory: &TempDir, name: &str) -> std::path::PathBuf {
    directory.path().join(name)
}

fn selector() -> Selector {
    Selector::All {
        source: SourceId::Codex,
        root: "/raw/sessions".to_owned(),
    }
}

fn session(native: &str, file: &str, title: &str) -> SessionWrite {
    SessionWrite {
        identity: SessionIdentity::new(SourceId::Codex, native),
        session_uuid: native.to_owned(),
        file_path: file.to_owned(),
        source_root: "/raw/sessions".to_owned(),
        title: title.to_owned(),
        summary_text: "stored summary".to_owned(),
        compact_text: "compact migration evidence".to_owned(),
        reasoning_summary_text: "reasoning projection".to_owned(),
        cwd: "/repo".to_owned(),
        model: "gpt-test".to_owned(),
        started_at: "2026-08-15T00:00:00Z".to_owned(),
        ended_at: "2026-08-15T00:01:00Z".to_owned(),
        path_date: "2026-08-15".to_owned(),
        raw_file_mtime: 1_700_000_000_000,
        raw_file_size: 2_048,
        index_version: INDEX_VERSION.to_owned(),
    }
}

fn message(seq: i64, role: MessageRole, body: &str) -> MessageWrite {
    MessageWrite {
        seq,
        role,
        timestamp: format!("2026-08-15T00:00:0{seq}Z"),
        source_kind: "event_msg".to_owned(),
        body_text: body.to_owned(),
        raw_start: Some((seq as u64) * 100),
        raw_end: Some((seq as u64) * 100 + 99),
        projection_epoch: PROJECTION_EPOCH,
    }
}

fn source_file(session: &SessionWrite, next_seq: i64) -> SourceFileState {
    SourceFileState {
        source_id: session.identity.source_id,
        file_path: session.file_path.clone(),
        source_root: session.source_root.clone(),
        source_generation: "generation-1".to_owned(),
        mtime_ms: 1_700_000_000_000.25,
        mtime_ns: Some(1_700_000_000_000_250_000),
        size: 2_048,
        indexed_bytes: 2_048,
        head_digest: "head".to_owned(),
        boundary_digest: "boundary".to_owned(),
        next_seq,
        reducer_checkpoint: Some(vec![1, 2, 3]),
        cwd: session.cwd.clone(),
        path_date: Some(session.path_date.clone()),
        extra_fingerprint: "accepted".to_owned(),
        projection_epoch: PROJECTION_EPOCH,
        analyzer_epoch: ANALYZER_EPOCH,
        coverage_epoch: COVERAGE_EPOCH,
        session: Some(session.identity.clone()),
    }
}

fn coverage(session_count: u64, document_count: u64) -> CoverageWrite {
    CoverageWrite {
        selector: selector(),
        source_fingerprint: "content".to_owned(),
        source_file_set_fingerprint: "file-set".to_owned(),
        source_file_count: session_count,
        indexed_session_count: session_count,
        indexed_document_count: document_count,
        source_generation: "generation-1".to_owned(),
        completed_at: Some("2026-08-15T00:02:00Z".to_owned()),
        index_version: INDEX_VERSION.to_owned(),
        projection_epoch: PROJECTION_EPOCH,
        analyzer_epoch: ANALYZER_EPOCH,
        coverage_epoch: COVERAGE_EPOCH,
    }
}

fn update_meta(path: &std::path::Path, key: &str, value: &str) {
    let connection = Connection::open(path).unwrap();
    assert_eq!(
        connection
            .execute("UPDATE meta SET value=? WHERE key=?", (value, key))
            .unwrap(),
        1
    );
}

fn meta_rows(path: &std::path::Path) -> Vec<(String, String)> {
    let connection = Connection::open(path).unwrap();
    let mut statement = connection
        .prepare("SELECT key, value FROM meta ORDER BY key")
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn expect_unsupported<T>(result: IndexResult<T>, axis: &str) {
    match result {
        Err(IndexError::UnsupportedSchema {
            user_version,
            detail,
        }) => {
            assert_eq!(user_version, SCHEMA_VERSION);
            assert!(
                detail.contains(axis),
                "unsupported-schema detail {detail:?} did not identify {axis:?}"
            );
        }
        Err(error) => panic!("expected typed UnsupportedSchema for {axis}, got {error}"),
        Ok(_) => panic!("expected {axis} mismatch to be rejected"),
    }
}

fn seed_coverage_fixture(path: &std::path::Path, native: &str) -> SessionWrite {
    let stored = session(
        native,
        &format!("/raw/sessions/{native}.jsonl"),
        "Epoch fixture",
    );
    let mut writer = IndexWriter::create_v8(path).unwrap();
    let mut transaction = writer.begin().unwrap();
    transaction
        .replace_session(
            &stored,
            &[message(0, MessageRole::User, "epoch searchable body")],
        )
        .unwrap();
    transaction.replace_coverage(&coverage(1, 2)).unwrap();
    transaction.commit().unwrap();
    drop(writer);
    stored
}

#[test]
fn v8_scratch_schema_fts_and_read_contract_work_together() {
    let directory = TempDir::new().unwrap();
    let path = db_path(&directory, "index.sqlite");
    let mut writer = IndexWriter::create_v8(&path).unwrap();
    let stored = session(
        "session-1",
        "/raw/sessions/2026/session-1.jsonl",
        "Migration plan",
    );
    let messages = [
        message(0, MessageRole::User, "请运行健康检查"),
        message(1, MessageRole::Assistant, "health check passed"),
    ];
    let mut transaction = writer.begin().unwrap();
    transaction.replace_session(&stored, &messages).unwrap();
    transaction
        .upsert_source_file(&source_file(&stored, 2))
        .unwrap();
    transaction.replace_coverage(&coverage(1, 3)).unwrap();
    transaction
        .upsert_cold_root(
            SourceId::Codex,
            "/cold/archive",
            Some("2026-08-15T00:00:00Z"),
        )
        .unwrap();
    assert_eq!(
        transaction.selector_counts(&selector()).unwrap(),
        SelectorCounts {
            session_count: 1,
            message_document_count: 2,
            document_count: 3,
        }
    );
    let receipt = transaction.commit().unwrap();
    assert!(receipt.invariants.is_valid());
    drop(writer);

    let reader = IndexReader::open(&path).unwrap();
    assert_eq!(reader.layout(), IndexLayout::V8);
    assert_eq!(reader.metadata().schema_version, SCHEMA_VERSION);
    assert_eq!(reader.metadata().analyzer_epoch, ANALYZER_EPOCH);
    assert_eq!(reader.selector_message_count(&selector()).unwrap(), 2);

    let stats = reader.stats(SourceId::Codex).unwrap();
    assert_eq!(stats.session_count, 1);
    assert_eq!(stats.message_count, 2);
    assert_eq!(stats.coverage.len(), 1);

    let listed = reader
        .list(&SessionListQuery {
            source_id: Some(SourceId::Codex),
            cwd: Some("REPO".to_owned()),
            since: Some("2026-08-01".to_owned()),
            selector: None,
            sort: SessionListSort::Ended,
            limit: 20,
        })
        .unwrap();
    assert_eq!(listed.len(), 1);

    let session_ref = SessionRef {
        source_id: SourceId::Codex,
        native_session_id: "session-1".to_owned(),
    };
    let page = reader.read_page(&session_ref, 0, 1).unwrap();
    assert_eq!(page.messages.len(), 1);
    assert!(page.has_more);
    assert_eq!(page.coverage.entries.len(), 1);
    let range = reader.read_range(&session_ref, 1, 1, 1).unwrap();
    assert_eq!(range.messages.len(), 2);
    assert_eq!(range.range_start_seq, 0);

    let candidates = reader
        .recall(&RecallSpec {
            terms: vec!["健康".to_owned()],
            like_needle: None,
            sources: vec![SourceId::Codex],
            session: None,
            selector: None,
            excluded_session_uuids: vec![],
            order: RecallOrder::Relevance,
            limit: 10,
        })
        .unwrap();
    assert!(
        candidates.iter().any(|candidate| {
            candidate.kind == DocumentKind::Message && candidate.seq == Some(0)
        })
    );
    let profile_candidates = reader
        .recall(&RecallSpec {
            terms: vec!["migration".to_owned()],
            like_needle: None,
            sources: vec![SourceId::Codex],
            session: None,
            selector: None,
            excluded_session_uuids: vec![],
            order: RecallOrder::Relevance,
            limit: 10,
        })
        .unwrap();
    assert!(
        profile_candidates
            .iter()
            .any(|candidate| candidate.kind == DocumentKind::SessionProfile)
    );

    let files = reader
        .source_files_for_paths(SourceId::Codex, std::slice::from_ref(&stored.file_path))
        .unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].next_seq, 2);
    assert_eq!(files[0].mtime_ns, Some(1_700_000_000_000_250_000));
    assert_eq!(
        reader.source_files_for_selector(&selector()).unwrap().len(),
        1
    );
    assert_eq!(reader.cold_roots(Some(SourceId::Codex)).unwrap().len(), 1);
    assert!(reader.ensure_invariants().unwrap().is_valid());

    let raw = Connection::open(&path).unwrap();
    assert_eq!(
        raw.query_row("SELECT mtime_ns FROM source_files", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        1_700_000_000_000_250_000
    );
    assert_eq!(
        raw.pragma_query_value::<i32, _>(None, "user_version", |row| row.get(0))
            .unwrap(),
        SCHEMA_VERSION
    );
    assert_eq!(
        raw.query_row(
            "SELECT type FROM sqlite_master WHERE name='session_rows'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        "table"
    );
    assert_eq!(
        raw.query_row(
            "SELECT type FROM sqlite_master WHERE name='sessions'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        "view"
    );
    assert!(
        raw.execute("UPDATE sessions SET title='legacy writer mutation'", [])
            .is_err(),
        "the public sessions compatibility view must stay read-only"
    );
    assert_eq!(
        raw.query_row("SELECT title FROM sessions", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap(),
        "Migration plan"
    );
}

#[test]
fn single_cjk_like_recall_is_bounded_and_pushes_all_scope_filters_into_sql() {
    let directory = TempDir::new().unwrap();
    let path = db_path(&directory, "like-recall.sqlite");
    let mut matching = session(
        "matching",
        "/scope/root/2026/matching.jsonl",
        "汉 profile match",
    );
    matching.source_root = "/scope/root".to_owned();
    matching.cwd = "/repo/target".to_owned();
    matching.path_date = "2026-08-15".to_owned();

    let mut excluded = session(
        "excluded",
        "/scope/root/2026/excluded.jsonl",
        "汉 excluded profile",
    );
    excluded.source_root = "/scope/root".to_owned();
    excluded.cwd = matching.cwd.clone();
    excluded.path_date = matching.path_date.clone();

    let mut wrong_root = session(
        "wrong-root",
        "/other/root/wrong-root.jsonl",
        "汉 wrong root",
    );
    wrong_root.source_root = "/other/root".to_owned();
    wrong_root.cwd = matching.cwd.clone();
    wrong_root.path_date = matching.path_date.clone();

    let mut wrong_cwd = session(
        "wrong-cwd",
        "/scope/root/2026/wrong-cwd.jsonl",
        "汉 wrong cwd",
    );
    wrong_cwd.source_root = "/scope/root".to_owned();
    wrong_cwd.cwd = "/repo/other".to_owned();
    wrong_cwd.path_date = matching.path_date.clone();

    let mut wrong_date = session(
        "wrong-date",
        "/scope/root/2025/wrong-date.jsonl",
        "汉 wrong date",
    );
    wrong_date.source_root = "/scope/root".to_owned();
    wrong_date.cwd = matching.cwd.clone();
    wrong_date.path_date = "2025-08-15".to_owned();

    let mut other_source = session(
        "other-source",
        "/scope/root/2026/other-source.jsonl",
        "汉 other source",
    );
    other_source.identity = SessionIdentity::new(SourceId::ClaudeCode, "other-source");
    other_source.source_root = "/scope/root".to_owned();
    other_source.cwd = matching.cwd.clone();
    other_source.path_date = matching.path_date.clone();

    let mut writer = IndexWriter::create_v8(&path).unwrap();
    let mut transaction = writer.begin().unwrap();
    for stored in [
        &matching,
        &excluded,
        &wrong_root,
        &wrong_cwd,
        &wrong_date,
        &other_source,
    ] {
        transaction
            .replace_session(stored, &[message(0, MessageRole::User, "汉 message match")])
            .unwrap();
    }
    transaction.commit().unwrap();
    drop(writer);

    let reader = IndexReader::open(&path).unwrap();
    let scoped_selector = Selector::CwdDateRange {
        source: SourceId::Codex,
        root: "/scope/root".to_owned(),
        cwd: matching.cwd.clone(),
        from_date: "2026-08-15".to_owned(),
        to_date: "2026-08-15".to_owned(),
    };
    let scoped = RecallSpec {
        terms: vec![],
        like_needle: Some("汉".to_owned()),
        sources: vec![SourceId::Codex],
        session: None,
        selector: Some(scoped_selector.clone()),
        excluded_session_uuids: vec![excluded.session_uuid.clone()],
        order: RecallOrder::Relevance,
        limit: 20,
    };
    let candidates = reader.recall(&scoped).unwrap();
    assert_eq!(candidates.len(), 2);
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.session_uuid == matching.session_uuid)
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.kind == DocumentKind::Message)
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.kind == DocumentKind::SessionProfile)
    );

    let mut limited = scoped.clone();
    limited.limit = 1;
    assert_eq!(reader.recall(&limited).unwrap().len(), 1);

    let mut session_scoped = scoped.clone();
    session_scoped.session = Some(matching.identity.as_session_ref());
    session_scoped.excluded_session_uuids.clear();
    assert_eq!(reader.recall(&session_scoped).unwrap().len(), 2);
    session_scoped
        .excluded_session_uuids
        .push(matching.session_uuid.clone());
    assert!(reader.recall(&session_scoped).unwrap().is_empty());

    let source_only = RecallSpec {
        terms: vec![],
        like_needle: Some("汉".to_owned()),
        sources: vec![SourceId::ClaudeCode],
        session: None,
        selector: None,
        excluded_session_uuids: vec![],
        order: RecallOrder::Relevance,
        limit: 20,
    };
    let candidates = reader.recall(&source_only).unwrap();
    assert_eq!(candidates.len(), 2);
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.source_id == SourceId::ClaudeCode)
    );

    let root_selector = RecallSpec {
        terms: vec![],
        like_needle: Some("汉".to_owned()),
        sources: vec![SourceId::Codex],
        session: None,
        selector: Some(Selector::All {
            source: SourceId::Codex,
            root: std::path::MAIN_SEPARATOR.to_string(),
        }),
        excluded_session_uuids: vec![],
        order: RecallOrder::Relevance,
        limit: 20,
    };
    let candidates = reader.recall(&root_selector).unwrap();
    assert_eq!(candidates.len(), 10);
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.session_uuid == wrong_root.session_uuid)
    );
}

#[test]
fn append_requires_the_exact_cursor_and_keeps_profile_and_counts_atomic() {
    let directory = TempDir::new().unwrap();
    let path = db_path(&directory, "append.sqlite");
    let mut writer = IndexWriter::create_v8(&path).unwrap();
    let mut stored = session("append", "/raw/sessions/append.jsonl", "before");
    let mut transaction = writer.begin().unwrap();
    transaction
        .replace_session(&stored, &[message(0, MessageRole::User, "first")])
        .unwrap();
    transaction.commit().unwrap();

    stored.title = "after".to_owned();
    let mut transaction = writer.begin().unwrap();
    transaction
        .append_session(&stored, 1, &[message(1, MessageRole::Assistant, "second")])
        .unwrap();
    transaction.commit().unwrap();

    let mut transaction = writer.begin().unwrap();
    let error = transaction
        .append_messages(
            &stored.identity,
            1,
            &[message(1, MessageRole::Assistant, "duplicate")],
        )
        .unwrap_err();
    assert!(error.to_string().contains("cursor mismatch"));
    transaction.rollback().unwrap();
    drop(writer);

    let reader = IndexReader::open(&path).unwrap();
    let session_ref = stored.identity.as_session_ref();
    let page = reader.read_page(&session_ref, 0, 20).unwrap();
    assert_eq!(page.total_count, 2);
    assert_eq!(page.session.title, "after");
    assert!(reader.ensure_invariants().unwrap().is_valid());
}

#[test]
fn cold_only_stored_projection_can_copy_without_raw_or_source_file_rows() {
    let directory = TempDir::new().unwrap();
    let source_path = db_path(&directory, "source.sqlite");
    let destination_path = db_path(&directory, "destination.sqlite");
    let stored = session(
        "cold-only",
        "/missing/hot/raw/cold-only.jsonl",
        "Cold migration",
    );
    let session_ref = stored.identity.as_session_ref();

    let mut source_writer = IndexWriter::create_v8(&source_path).unwrap();
    let mut transaction = source_writer.begin().unwrap();
    transaction
        .replace_session(
            &stored,
            &[message(0, MessageRole::User, "唯一留存在索引里的证据")],
        )
        .unwrap();
    transaction.commit().unwrap();
    drop(source_writer);

    let source_reader = IndexReader::open(&source_path).unwrap();
    let bundle = source_reader.export_session_bundle(&session_ref).unwrap();
    assert!(bundle.source_files.is_empty());

    let mut destination_writer = IndexWriter::create_v8(&destination_path).unwrap();
    let mut transaction = destination_writer.begin().unwrap();
    transaction.copy_session_bundle(&bundle).unwrap();
    transaction.commit().unwrap();
    drop(destination_writer);

    let destination = IndexReader::open(&destination_path).unwrap();
    assert_eq!(destination.stats(SourceId::Codex).unwrap().session_count, 1);
    assert_eq!(
        destination
            .read_page(&session_ref, 0, 20)
            .unwrap()
            .total_count,
        1
    );
    assert!(
        destination
            .source_files_for_paths(SourceId::Codex, std::slice::from_ref(&stored.file_path))
            .unwrap()
            .is_empty()
    );
    assert!(destination.ensure_invariants().unwrap().is_valid());
}

#[test]
fn prune_retains_hot_and_cold_then_deletes_unretained_projection_and_fts() {
    let directory = TempDir::new().unwrap();
    let path = db_path(&directory, "prune.sqlite");
    let hot = session("hot", "/raw/sessions/hot.jsonl", "hot");
    let cold = session("cold", "/raw/sessions/cold.jsonl", "cold");
    let mut writer = IndexWriter::create_v8(&path).unwrap();
    let mut transaction = writer.begin().unwrap();
    transaction
        .replace_session(&hot, &[message(0, MessageRole::User, "hot evidence")])
        .unwrap();
    transaction
        .replace_session(&cold, &[message(0, MessageRole::User, "cold evidence")])
        .unwrap();
    let outcome = transaction
        .prune(
            &selector(),
            &HashSet::from([hot.file_path.clone()]),
            &HashSet::from([cold.identity.native_session_id.clone()]),
        )
        .unwrap();
    assert_eq!(outcome.removed, 0);
    assert_eq!(outcome.retained_cold, 1);
    transaction.commit().unwrap();

    let mut transaction = writer.begin().unwrap();
    let outcome = transaction
        .prune(
            &selector(),
            &HashSet::from([hot.file_path.clone()]),
            &HashSet::new(),
        )
        .unwrap();
    assert_eq!(outcome.removed, 1);
    transaction.commit().unwrap();
    drop(writer);

    let reader = IndexReader::open(&path).unwrap();
    assert_eq!(reader.stats(SourceId::Codex).unwrap().session_count, 1);
    assert!(reader.ensure_invariants().unwrap().is_valid());
}

#[test]
fn coverage_invalidation_drops_every_same_source_root_proof_and_preserves_other_namespaces() {
    let directory = TempDir::new().unwrap();
    let path = db_path(&directory, "invalidate-coverage.sqlite");
    let requested = Selector::CwdDateRange {
        source: SourceId::Codex,
        root: "/raw/sessions".to_owned(),
        cwd: "/repo/target".to_owned(),
        from_date: "2026-08-15".to_owned(),
        to_date: "2026-08-15".to_owned(),
    };
    let broad = Selector::All {
        source: SourceId::Codex,
        root: "/raw/sessions".to_owned(),
    };
    let sibling_cwd = Selector::Cwd {
        source: SourceId::Codex,
        root: "/raw/sessions".to_owned(),
        cwd: "/repo/other".to_owned(),
    };
    let unrelated_root = Selector::All {
        source: SourceId::Codex,
        root: "/other/sessions".to_owned(),
    };
    let unrelated_source = Selector::All {
        source: SourceId::ClaudeCode,
        root: "/raw/sessions".to_owned(),
    };

    let mut writer = IndexWriter::create_v8(&path).unwrap();
    let mut transaction = writer.begin().unwrap();
    for stored_selector in [
        broad.clone(),
        requested.clone(),
        sibling_cwd,
        unrelated_root.clone(),
        unrelated_source.clone(),
    ] {
        let mut record = coverage(0, 0);
        record.selector = stored_selector;
        transaction.replace_coverage(&record).unwrap();
    }

    assert_eq!(
        transaction
            .invalidate_covering_coverage(&requested)
            .unwrap(),
        3
    );
    assert_eq!(
        transaction
            .invalidate_covering_coverage(&requested)
            .unwrap(),
        0
    );
    transaction.commit().unwrap();
    drop(writer);

    let reader = IndexReader::open(&path).unwrap();
    let remaining = reader
        .coverage_records(SourceId::Codex)
        .unwrap()
        .into_iter()
        .map(|record| record.selector.storage_key())
        .collect::<HashSet<_>>();
    assert_eq!(remaining, HashSet::from([unrelated_root.storage_key()]));
    assert_eq!(
        reader
            .coverage_records(SourceId::ClaudeCode)
            .unwrap()
            .into_iter()
            .map(|record| record.selector.storage_key())
            .collect::<HashSet<_>>(),
        HashSet::from([unrelated_source.storage_key()])
    );
}

#[test]
fn broad_partial_invalidation_removes_preexisting_narrow_proofs() {
    let directory = TempDir::new().unwrap();
    let path = db_path(&directory, "invalidate-broad-coverage.sqlite");
    let broad = Selector::All {
        source: SourceId::Codex,
        root: "/raw/sessions".to_owned(),
    };
    let narrow = Selector::Cwd {
        source: SourceId::Codex,
        root: "/raw/sessions".to_owned(),
        cwd: "/repo/target".to_owned(),
    };

    let mut writer = IndexWriter::create_v8(&path).unwrap();
    let mut transaction = writer.begin().unwrap();
    let mut record = coverage(0, 0);
    record.selector = narrow;
    transaction.replace_coverage(&record).unwrap();

    assert_eq!(transaction.invalidate_covering_coverage(&broad).unwrap(), 1);
    transaction.commit().unwrap();
    drop(writer);

    assert!(
        IndexReader::open(&path)
            .unwrap()
            .coverage_records(SourceId::Codex)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn v7_reader_uses_legacy_sessions_messages_and_coverage_without_writes() {
    let directory = TempDir::new().unwrap();
    let path = db_path(&directory, "v7.sqlite");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            r#"
            CREATE TABLE sessions (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              source_id TEXT NOT NULL,
              native_session_id TEXT NOT NULL,
              session_key TEXT NOT NULL UNIQUE,
              session_uuid TEXT NOT NULL,
              file_path TEXT NOT NULL,
              source_root TEXT NOT NULL,
              title TEXT NOT NULL,
              summary_text TEXT NOT NULL,
              compact_text TEXT NOT NULL,
              reasoning_summary_text TEXT NOT NULL,
              cwd TEXT NOT NULL,
              model TEXT NOT NULL,
              started_at TEXT NOT NULL,
              ended_at TEXT NOT NULL,
              path_date TEXT NOT NULL,
              message_count INTEGER NOT NULL,
              raw_file_mtime INTEGER NOT NULL,
              raw_file_size INTEGER NOT NULL,
              index_version TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            CREATE TABLE messages (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
              session_uuid TEXT NOT NULL,
              seq INTEGER NOT NULL,
              role TEXT NOT NULL,
              content_text TEXT NOT NULL,
              timestamp TEXT NOT NULL,
              source_kind TEXT NOT NULL,
              UNIQUE(session_id, seq)
            );
            CREATE TABLE coverage (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              source_id TEXT NOT NULL,
              selector_key TEXT NOT NULL UNIQUE,
              selector_json TEXT NOT NULL,
              selector_kind TEXT NOT NULL,
              root TEXT NOT NULL,
              cwd TEXT,
              from_date TEXT,
              to_date TEXT,
              source_fingerprint TEXT NOT NULL,
              source_file_set_fingerprint TEXT NOT NULL,
              source_file_count INTEGER NOT NULL,
              indexed_session_count INTEGER NOT NULL,
              completed_at TEXT NOT NULL,
              index_version TEXT NOT NULL
            );
            INSERT INTO sessions VALUES (
              1, 'codex', 'legacy', 'codex:legacy', 'legacy',
              '/raw/sessions/legacy.jsonl', '/raw/sessions', 'Legacy', 'summary',
              'compact', 'reasoning', '/repo', 'model', '2026-01-01T00:00:00Z',
              '2026-01-01T00:01:00Z', '2026-01-01', 2, 1, 100,
              'shlog-v7-source-identity', '2026-01-01T00:02:00Z'
            );
            INSERT INTO messages(session_id, session_uuid, seq, role, content_text, timestamp, source_kind)
              VALUES (1, 'legacy', 0, 'user', 'hello', '2026-01-01T00:00:00Z', 'event_msg'),
                     (1, 'legacy', 1, 'assistant', 'world', '2026-01-01T00:00:01Z', 'event_msg');
            INSERT INTO coverage(
              source_id, selector_key, selector_json, selector_kind, root,
              source_fingerprint, source_file_set_fingerprint, source_file_count,
              indexed_session_count, completed_at, index_version
            ) VALUES (
              'codex',
              '{"kind":"all","source":"codex","root":"/raw/sessions"}',
              '{"kind":"all","source":"codex","root":"/raw/sessions"}',
              'all', '/raw/sessions', 'content', 'files', 1, 1,
              '2026-01-01T00:02:00Z', 'shlog-v7-source-identity'
            );
            "#,
        )
        .unwrap();
    drop(connection);

    let reader = IndexReader::open(&path).unwrap();
    assert_eq!(reader.layout(), IndexLayout::V7);
    // Status-oriented proof reads keep working on legacy layout so `status`
    // can nudge the user toward the explicit sync migration.
    assert_eq!(reader.stats(SourceId::Codex).unwrap().message_count, 2);
    assert!(!reader.coverage_status(&selector()).unwrap().complete);
    let session_ref = SessionRef {
        source_id: SourceId::Codex,
        native_session_id: "legacy".to_owned(),
    };

    // Content-bearing commands fail closed on v7 with the typed upgrade
    // error; the only v7 consumer is the explicit sync migration.
    let read_error = reader.read_page(&session_ref, 1, 20).unwrap_err();
    assert!(matches!(
        &read_error,
        IndexError::UnsupportedSchema { detail, .. } if detail.contains("shlog sync")
    ));
    let recall_error = reader
        .recall(&RecallSpec {
            terms: vec!["legacy".to_owned()],
            like_needle: None,
            sources: vec![SourceId::Codex],
            session: Some(session_ref.clone()),
            selector: None,
            excluded_session_uuids: vec![],
            order: RecallOrder::Relevance,
            limit: 10,
        })
        .unwrap_err();
    assert!(matches!(
        &recall_error,
        IndexError::UnsupportedSchema { .. }
    ));
    let bundle_error = reader.export_session_bundle(&session_ref).unwrap_err();
    assert!(matches!(
        &bundle_error,
        IndexError::UnsupportedSchema { .. }
    ));
    let invariant_error = reader.ensure_invariants().unwrap_err();
    assert!(matches!(
        &invariant_error,
        IndexError::UnsupportedSchema { .. }
    ));
}

#[test]
fn invariant_audit_detects_logical_count_corruption() {
    let directory = TempDir::new().unwrap();
    let path = db_path(&directory, "invalid.sqlite");
    let stored = session("invalid", "/raw/sessions/invalid.jsonl", "invalid");
    let mut writer = IndexWriter::create_v8(&path).unwrap();
    let mut transaction = writer.begin().unwrap();
    transaction
        .replace_session(&stored, &[message(0, MessageRole::User, "body")])
        .unwrap();
    transaction.commit().unwrap();
    drop(writer);

    let connection = Connection::open(&path).unwrap();
    connection
        .execute("UPDATE session_rows SET message_count=99", [])
        .unwrap();
    drop(connection);

    let reader = IndexReader::open(&path).unwrap();
    let report = reader.check_invariants().unwrap();
    assert!(!report.is_valid());
    assert!(
        report
            .violations
            .iter()
            .any(|value| value.contains("message_count=99"))
    );
}

#[test]
fn incompatible_v8_content_metadata_is_typed_and_fail_closed() {
    let directory = TempDir::new().unwrap();
    for (name, axis, value) in [
        ("projection.sqlite", "projection_epoch", "999"),
        ("analyzer.sqlite", "analyzer_epoch", "999"),
        ("version.sqlite", "index_version", "shlog-v9-future"),
    ] {
        let path = db_path(&directory, name);
        drop(IndexWriter::create_v8(&path).unwrap());
        update_meta(&path, axis, value);
        let before = meta_rows(&path);

        expect_unsupported(IndexReader::open(&path), axis);
        expect_unsupported(IndexWriter::open_v8(&path), axis);

        assert_eq!(
            meta_rows(&path),
            before,
            "failed open must not rewrite incompatible metadata"
        );
    }
}

#[test]
fn stale_global_coverage_epoch_keeps_content_readable_but_blocks_writes() {
    let directory = TempDir::new().unwrap();
    let path = db_path(&directory, "stale-global-coverage.sqlite");
    let stored = seed_coverage_fixture(&path, "coverage-global");
    update_meta(&path, "coverage_epoch", "999");
    let before = meta_rows(&path);

    let reader = IndexReader::open(&path).unwrap();
    assert_eq!(reader.metadata().coverage_epoch, 999);
    let stats = reader.stats(SourceId::Codex).unwrap();
    assert_eq!(stats.session_count, 1);
    assert_eq!(stats.message_count, 1);
    assert!(stats.coverage.is_empty());
    let page = reader
        .read_page(&stored.identity.as_session_ref(), 0, 20)
        .unwrap();
    assert_eq!(page.messages.len(), 1);
    assert!(page.coverage.entries.is_empty());
    assert!(
        reader
            .recall(&RecallSpec {
                terms: vec!["searchable".to_owned()],
                like_needle: None,
                sources: vec![SourceId::Codex],
                session: None,
                selector: None,
                excluded_session_uuids: vec![],
                order: RecallOrder::Relevance,
                limit: 10,
            })
            .unwrap()
            .iter()
            .any(|candidate| candidate.body_text.contains("searchable"))
    );
    let status = reader.coverage_status(&selector()).unwrap();
    assert!(!status.complete);
    assert!(status.covering_selectors.is_empty());
    drop(reader);

    expect_unsupported(IndexWriter::open_v8(&path), "coverage_epoch");
    assert_eq!(meta_rows(&path), before);
}

#[test]
fn stale_row_level_coverage_is_not_reported_as_current() {
    let directory = TempDir::new().unwrap();
    let path = db_path(&directory, "stale-row-coverage.sqlite");
    let stored = seed_coverage_fixture(&path, "coverage-row");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute("UPDATE coverage SET coverage_epoch=?", [COVERAGE_EPOCH + 1])
        .unwrap();
    drop(connection);

    let reader = IndexReader::open(&path).unwrap();
    assert_eq!(
        reader
            .read_page(&stored.identity.as_session_ref(), 0, 20)
            .unwrap()
            .messages
            .len(),
        1
    );
    assert!(reader.coverage_records(SourceId::Codex).unwrap().is_empty());
    let status = reader.coverage_status(&selector()).unwrap();
    assert!(!status.complete);
    assert!(status.covering_selectors.is_empty());
}

#[test]
fn writer_rejects_mixed_generation_payloads_without_normalizing_them() {
    let directory = TempDir::new().unwrap();
    let path = db_path(&directory, "mixed-generation-write.sqlite");
    let mut writer = IndexWriter::create_v8(&path).unwrap();
    let stored = session("mixed", "/raw/sessions/mixed.jsonl", "mixed");
    let mut transaction = writer.begin().unwrap();

    let mut bad_session = stored.clone();
    bad_session.index_version = "shlog-v9-future".to_owned();
    assert!(
        transaction
            .replace_session(&bad_session, &[])
            .unwrap_err()
            .to_string()
            .contains("session index_version")
    );

    let mut bad_message = message(0, MessageRole::User, "wrong projection");
    bad_message.projection_epoch = PROJECTION_EPOCH + 1;
    assert!(
        transaction
            .replace_session(&stored, &[bad_message])
            .unwrap_err()
            .to_string()
            .contains("message projection_epoch")
    );

    let mut bad_source_file = source_file(&stored, 0);
    bad_source_file.analyzer_epoch = ANALYZER_EPOCH + 1;
    assert!(
        transaction
            .upsert_source_file(&bad_source_file)
            .unwrap_err()
            .to_string()
            .contains("source file epochs")
    );
    bad_source_file.analyzer_epoch = ANALYZER_EPOCH;
    bad_source_file.mtime_ns = Some(-1);
    assert!(
        transaction
            .upsert_source_file(&bad_source_file)
            .unwrap_err()
            .to_string()
            .contains("mtime_ns")
    );

    let mut bad_coverage = coverage(0, 0);
    bad_coverage.index_version = "shlog-v9-future".to_owned();
    assert!(
        transaction
            .replace_coverage(&bad_coverage)
            .unwrap_err()
            .to_string()
            .contains("coverage index_version")
    );
    transaction.rollback().unwrap();
    drop(writer);

    let reader = IndexReader::open(&path).unwrap();
    assert_eq!(reader.stats(SourceId::Codex).unwrap().session_count, 0);
    assert_eq!(reader.metadata().projection_epoch, PROJECTION_EPOCH);
    assert_eq!(reader.metadata().analyzer_epoch, ANALYZER_EPOCH);
    assert_eq!(reader.metadata().coverage_epoch, COVERAGE_EPOCH);
    assert_eq!(reader.metadata().index_version, INDEX_VERSION);
}

#[test]
fn bundle_copy_rejects_incompatible_projection_instead_of_normalizing_it() {
    let directory = TempDir::new().unwrap();
    let source_path = db_path(&directory, "bundle-source.sqlite");
    let destination_path = db_path(&directory, "bundle-destination.sqlite");
    let stored = seed_coverage_fixture(&source_path, "bundle-generation");
    let source = IndexReader::open(&source_path).unwrap();
    let mut bundle = source
        .export_session_bundle(&stored.identity.as_session_ref())
        .unwrap();
    bundle
        .documents
        .iter_mut()
        .find(|document| document.kind == DocumentKind::SessionProfile)
        .unwrap()
        .projection_epoch = 0;

    let mut writer = IndexWriter::create_v8(&destination_path).unwrap();
    let mut transaction = writer.begin().unwrap();
    assert!(
        transaction
            .copy_session_bundle(&bundle)
            .unwrap_err()
            .to_string()
            .contains("stored profile projection_epoch")
    );
    transaction.rollback().unwrap();
    drop(writer);

    let destination = IndexReader::open(&destination_path).unwrap();
    assert_eq!(destination.stats(SourceId::Codex).unwrap().session_count, 0);
}

#[test]
fn scratch_create_refuses_to_overwrite_any_existing_file() {
    let directory = TempDir::new().unwrap();
    let path = db_path(&directory, "existing.sqlite");
    std::fs::write(&path, b"do not replace").unwrap();
    assert!(matches!(
        IndexWriter::create_v8(&path),
        Err(IndexError::ScratchExists(found)) if found == path
    ));
    assert_eq!(std::fs::read(&path).unwrap(), b"do not replace");
}

#[test]
fn pre_guard_v8_layout_is_rejected_instead_of_opened_for_writing() {
    let directory = TempDir::new().unwrap();
    let path = db_path(&directory, "unguarded-v8.sqlite");
    drop(IndexWriter::create_v8(&path).unwrap());

    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "PRAGMA foreign_keys=OFF; \
             DROP VIEW sessions; \
             ALTER TABLE session_rows RENAME TO sessions;",
        )
        .unwrap();
    drop(connection);

    let error = match IndexWriter::open_v8(&path) {
        Ok(_) => panic!("unguarded v8 layout must not open for writing"),
        Err(error) => error,
    };
    assert!(matches!(error, IndexError::UnsupportedSchema { .. }));
    assert!(error.to_string().contains("read-only sessions view"));
}

#[test]
fn v8_without_exact_mtime_column_is_rejected_as_unsupported() {
    let directory = TempDir::new().unwrap();
    let path = db_path(&directory, "missing-mtime-ns.sqlite");
    drop(IndexWriter::create_v8(&path).unwrap());

    let connection = Connection::open(&path).unwrap();
    connection
        .execute("ALTER TABLE source_files DROP COLUMN mtime_ns", [])
        .unwrap();
    drop(connection);

    expect_unsupported(IndexReader::open(&path), "mtime_ns");
    expect_unsupported(IndexWriter::open_v8(&path), "mtime_ns");
}

#[cfg(unix)]
#[test]
fn scratch_database_and_sqlite_sidecars_are_private() {
    let directory = TempDir::new().unwrap();
    let path = db_path(&directory, "private.sqlite");
    let writer = IndexWriter::create_v8(&path).unwrap();

    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "new index DB must not be group/world readable");

    for suffix in ["-wal", "-shm"] {
        let sidecar = std::path::PathBuf::from(format!("{}{suffix}", path.display()));
        if sidecar.exists() {
            let sidecar_mode = std::fs::metadata(&sidecar).unwrap().permissions().mode() & 0o777;
            assert_eq!(
                sidecar_mode & 0o077,
                0,
                "SQLite sidecar {} must not be group/world accessible",
                sidecar.display()
            );
        }
    }
    drop(writer);
}

#[test]
fn v8_writers_persist_delete_journal_mode_without_sidecars() {
    let directory = TempDir::new().unwrap();
    let path = db_path(&directory, "single-file.sqlite");
    drop(IndexWriter::create_v8(&path).unwrap());

    let connection = Connection::open(&path).unwrap();
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal_mode.to_ascii_lowercase(), "delete");
    drop(connection);

    for suffix in ["-journal", "-wal", "-shm"] {
        assert!(
            !std::path::PathBuf::from(format!("{}{suffix}", path.display())).exists(),
            "quiescent v8 index must not retain a SQLite {suffix} sidecar"
        );
    }
}

#[test]
fn opening_an_existing_wal_v8_for_writing_converts_it_to_delete_mode() {
    let directory = TempDir::new().unwrap();
    let path = db_path(&directory, "existing-wal.sqlite");
    drop(IndexWriter::create_v8(&path).unwrap());

    let connection = Connection::open(&path).unwrap();
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
    drop(connection);

    drop(IndexWriter::open_v8(&path).unwrap());
    let connection = Connection::open(&path).unwrap();
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal_mode.to_ascii_lowercase(), "delete");
    drop(connection);

    for suffix in ["-journal", "-wal", "-shm"] {
        assert!(
            !std::path::PathBuf::from(format!("{}{suffix}", path.display())).exists(),
            "WAL compatibility conversion must remove SQLite {suffix} sidecar"
        );
    }
}
