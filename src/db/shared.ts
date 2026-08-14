import type BetterSqlite3 from "better-sqlite3";

export type Db = BetterSqlite3.Database;
export type SqlParams = unknown[];

export const BUSY_TIMEOUT_MS = 5000;
