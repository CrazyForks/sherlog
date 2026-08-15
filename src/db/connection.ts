import { existsSync } from "node:fs";
import { DatabaseSync } from "node:sqlite";
import { IndexSchemaUpgradeRequiredError, IndexUnavailableError } from "./errors";
import { ensureSchema } from "./schema";
import { BUSY_TIMEOUT_MS, pragmaValue, setPragma, type Db } from "./shared";

export { IndexSchemaUpgradeRequiredError, IndexUnavailableError } from "./errors";

let sqliteOpened = false;

/** True only after a read/write connection has actually opened the SQLite db. */
export function sqliteNativeModuleLoaded(): boolean {
  return sqliteOpened;
}

export function openReadDb(dbPath: string): Db {
  if (!existsSync(dbPath)) {
    throw new IndexUnavailableError(dbPath);
  }

  const db = new DatabaseSync(dbPath, { readOnly: true });
  sqliteOpened = true;
  setPragma(db, `busy_timeout = ${BUSY_TIMEOUT_MS}`);
  setPragma(db, "query_only = ON");
  setPragma(db, "temp_store = MEMORY");
  return db;
}

// Why: callers used to do `const db = openReadDb(...); ... db.close();` which
// leaks the connection if work in between throws. Wrapping in try/finally at
// every callsite is noise — fold it once.
export function withReadDb<T>(dbPath: string, fn: (db: Db) => T): T {
  const db = openReadDb(dbPath);
  try {
    return fn(db);
  } finally {
    db.close();
  }
}

export function withSourceAwareReadDb<T>(dbPath: string, fn: (db: Db) => T): T {
  const db = openReadDb(dbPath);
  try {
    assertSourceAwareReadSchema(db, dbPath);
    return fn(db);
  } finally {
    db.close();
  }
}

export function openWriteDb(dbPath: string): Db {
  const db = new DatabaseSync(dbPath);
  sqliteOpened = true;
  setPragma(db, `busy_timeout = ${BUSY_TIMEOUT_MS}`);
  pragmaValue(db, "journal_mode = WAL");
  setPragma(db, "synchronous = NORMAL");
  setPragma(db, "temp_store = MEMORY");
  setPragma(db, "foreign_keys = ON");
  ensureSchema(db);
  return db;
}

function assertSourceAwareReadSchema(db: Db, dbPath: string): void {
  const requiredColumns = [
    ["sessions", "source_id"],
    ["sessions", "native_session_id"],
    ["sessions", "session_key"],
    ["coverage", "source_id"],
  ];
  const missingColumns = requiredColumns
    .filter(([tableName, columnName]) => !tableColumnExists(db, tableName, columnName))
    .map(([tableName, columnName]) => `${tableName}.${columnName}`);

  if (missingColumns.length > 0) {
    throw new IndexSchemaUpgradeRequiredError(dbPath, missingColumns);
  }
}

function tableColumnExists(db: Db, tableName: string, columnName: string): boolean {
  return db
    .prepare(`
      SELECT name
      FROM pragma_table_info(?)
      WHERE name = ?
      LIMIT 1
    `)
    .get(tableName, columnName) !== undefined;
}
