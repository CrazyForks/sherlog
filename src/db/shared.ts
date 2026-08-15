import type { DatabaseSync, SQLInputValue } from "node:sqlite";

export type Db = DatabaseSync;
export type SqlParams = SQLInputValue[];

export const BUSY_TIMEOUT_MS = 5000;

/**
 * Run a no-result PRAGMA statement (e.g. `busy_timeout`, `foreign_keys`,
 * `query_only`, `temp_store`, `synchronous`). node:sqlite has no
 * `db.pragma()` helper, so assignment pragmas go through `exec`.
 */
export function setPragma(db: Db, sql: string): void {
  db.exec(`PRAGMA ${sql}`);
}

/**
 * Run a row-returning PRAGMA statement (e.g. `journal_mode = WAL`,
 * `wal_checkpoint(TRUNCATE)`) and return its result row.
 */
export function pragmaValue(db: Db, sql: string): Record<string, unknown> | undefined {
  return db.prepare(`PRAGMA ${sql}`).get() as Record<string, unknown> | undefined;
}

/**
 * Minimal replacement for better-sqlite3's `db.transaction()` helper on top
 * of node:sqlite. The outermost call opens BEGIN IMMEDIATE (busy database
 * fails fast instead of deferring into a busy-timeout loop); nested calls —
 * which better-sqlite3 maps to savepoints — map to SAVEPOINT/RELEASE the
 * same way.
 */
const transactionDepth = new WeakMap<Db, number>();

export function withTransaction<T>(db: Db, fn: () => T): T {
  const depth = transactionDepth.get(db) ?? 0;
  if (depth === 0) {
    db.exec("BEGIN IMMEDIATE");
  } else {
    db.exec(`SAVEPOINT shlog_nested_${depth}`);
  }
  transactionDepth.set(db, depth + 1);
  try {
    const result = fn();
    if (depth === 0) {
      db.exec("COMMIT");
    } else {
      db.exec(`RELEASE shlog_nested_${depth}`);
    }
    transactionDepth.set(db, depth);
    return result;
  } catch (error) {
    try {
      if (depth === 0) {
        db.exec("ROLLBACK");
      } else {
        db.exec(`ROLLBACK TO shlog_nested_${depth}`);
        db.exec(`RELEASE shlog_nested_${depth}`);
      }
    } catch {
      // Transaction may already be rolled back; preserve the original error.
    }
    transactionDepth.set(db, depth);
    throw error;
  }
}
