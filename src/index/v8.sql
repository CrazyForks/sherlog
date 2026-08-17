CREATE TABLE meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
) WITHOUT ROWID;

-- `session_rows` is intentionally not named `sessions`.  The latter is the
-- stable public read surface below.  A legacy TypeScript writer that opens a
-- v8 database reaches `UPDATE sessions ...` during ensureSchema and fails
-- closed because the view has no INSTEAD OF triggers.
CREATE TABLE session_rows (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  source_id TEXT NOT NULL DEFAULT 'codex',
  native_session_id TEXT NOT NULL DEFAULT '',
  session_key TEXT NOT NULL UNIQUE,
  session_uuid TEXT NOT NULL,
  file_path TEXT NOT NULL,
  source_root TEXT NOT NULL DEFAULT '',
  title TEXT NOT NULL DEFAULT '',
  summary_text TEXT NOT NULL DEFAULT '',
  compact_text TEXT NOT NULL DEFAULT '',
  reasoning_summary_text TEXT NOT NULL DEFAULT '',
  cwd TEXT NOT NULL DEFAULT '',
  model TEXT NOT NULL DEFAULT '',
  started_at TEXT NOT NULL,
  ended_at TEXT NOT NULL,
  path_date TEXT NOT NULL DEFAULT '',
  message_count INTEGER NOT NULL DEFAULT 0 CHECK (message_count >= 0),
  document_count INTEGER NOT NULL DEFAULT 0 CHECK (document_count >= 0),
  raw_file_mtime INTEGER NOT NULL DEFAULT 0,
  raw_file_size INTEGER NOT NULL DEFAULT 0 CHECK (raw_file_size >= 0),
  index_version TEXT NOT NULL DEFAULT '',
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(source_id, native_session_id),
  UNIQUE(source_id, file_path)
);

CREATE INDEX idx_sessions_started_at ON session_rows(started_at DESC);
CREATE INDEX idx_sessions_source_started_at ON session_rows(source_id, started_at DESC);
CREATE INDEX idx_sessions_source_ended_at ON session_rows(source_id, ended_at DESC);

CREATE VIEW sessions AS
SELECT
  id,
  source_id,
  native_session_id,
  session_key,
  session_uuid,
  file_path,
  source_root,
  title,
  summary_text,
  compact_text,
  reasoning_summary_text,
  cwd,
  model,
  started_at,
  ended_at,
  path_date,
  message_count,
  document_count,
  raw_file_mtime,
  raw_file_size,
  index_version,
  updated_at
FROM session_rows;

CREATE TABLE source_files (
  source_id TEXT NOT NULL,
  file_path TEXT NOT NULL,
  session_id INTEGER REFERENCES session_rows(id) ON DELETE SET NULL,
  source_root TEXT NOT NULL DEFAULT '',
  source_generation TEXT NOT NULL DEFAULT '',
  mtime_ms REAL NOT NULL,
  mtime_ns INTEGER,
  size INTEGER NOT NULL CHECK (size >= 0),
  indexed_bytes INTEGER NOT NULL DEFAULT 0 CHECK (indexed_bytes >= 0 AND indexed_bytes <= size),
  head_digest TEXT NOT NULL DEFAULT '',
  boundary_digest TEXT NOT NULL DEFAULT '',
  next_seq INTEGER NOT NULL DEFAULT 0 CHECK (next_seq >= 0),
  reducer_checkpoint BLOB,
  cwd TEXT NOT NULL DEFAULT '',
  path_date TEXT,
  extra_fingerprint TEXT NOT NULL DEFAULT '',
  projection_epoch INTEGER NOT NULL,
  analyzer_epoch INTEGER NOT NULL,
  coverage_epoch INTEGER NOT NULL,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY(source_id, file_path)
) WITHOUT ROWID;

CREATE INDEX idx_source_files_session ON source_files(session_id);
CREATE INDEX idx_source_files_root ON source_files(source_id, source_root);

CREATE TABLE documents (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id INTEGER NOT NULL REFERENCES session_rows(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('message', 'session_profile')),
  seq INTEGER,
  role TEXT,
  timestamp TEXT,
  source_kind TEXT,
  body_text TEXT NOT NULL DEFAULT '',
  title_text TEXT NOT NULL DEFAULT '',
  summary_text TEXT NOT NULL DEFAULT '',
  compact_text TEXT NOT NULL DEFAULT '',
  reasoning_text TEXT NOT NULL DEFAULT '',
  raw_start INTEGER,
  raw_end INTEGER,
  projection_epoch INTEGER NOT NULL,
  CHECK (
    (kind = 'message' AND seq IS NOT NULL AND role IN ('user', 'assistant')) OR
    (kind = 'session_profile' AND seq IS NULL AND role IS NULL)
  ),
  CHECK (
    (raw_start IS NULL AND raw_end IS NULL) OR
    (raw_start IS NOT NULL AND raw_end IS NOT NULL AND raw_start >= 0 AND raw_end >= raw_start)
  )
);

CREATE UNIQUE INDEX idx_documents_message_seq
  ON documents(session_id, seq) WHERE kind = 'message';
CREATE UNIQUE INDEX idx_documents_one_profile
  ON documents(session_id) WHERE kind = 'session_profile';
CREATE INDEX idx_documents_session_kind_seq ON documents(session_id, kind, seq);

CREATE VIRTUAL TABLE documents_fts USING fts5(
  body_text,
  title_text,
  summary_text,
  compact_text,
  reasoning_text,
  content='',
  contentless_delete=1,
  tokenize='unicode61 remove_diacritics 1'
);

CREATE TABLE coverage (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  source_id TEXT NOT NULL DEFAULT 'codex',
  selector_key TEXT NOT NULL UNIQUE,
  selector_json TEXT NOT NULL,
  selector_kind TEXT NOT NULL,
  root TEXT NOT NULL,
  cwd TEXT,
  from_date TEXT,
  to_date TEXT,
  source_fingerprint TEXT NOT NULL,
  source_file_set_fingerprint TEXT NOT NULL DEFAULT '',
  source_file_count INTEGER NOT NULL CHECK (source_file_count >= 0),
  indexed_session_count INTEGER NOT NULL CHECK (indexed_session_count >= 0),
  indexed_document_count INTEGER NOT NULL DEFAULT 0 CHECK (indexed_document_count >= 0),
  source_generation TEXT NOT NULL DEFAULT '',
  completed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  index_version TEXT NOT NULL,
  projection_epoch INTEGER NOT NULL,
  analyzer_epoch INTEGER NOT NULL,
  coverage_epoch INTEGER NOT NULL
);

CREATE INDEX idx_coverage_root ON coverage(root);
CREATE INDEX idx_coverage_source_root ON coverage(source_id, root);

CREATE TABLE cold_roots (
  source_id TEXT NOT NULL,
  root TEXT NOT NULL,
  added_at TEXT NOT NULL,
  PRIMARY KEY(source_id, root)
) WITHOUT ROWID;

PRAGMA user_version = 8;
