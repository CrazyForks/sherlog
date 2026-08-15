export type { Db, SqlParams } from "./db/shared";
export { withTransaction } from "./db/shared";
export {
  IndexSchemaUpgradeRequiredError,
  IndexUnavailableError,
} from "./db/errors";
export {
  openReadDb,
  openWriteDb,
  sqliteNativeModuleLoaded,
  withReadDb,
  withSourceAwareReadDb,
} from "./db/connection";
export {
  getIndexedSessionMeta,
  getIndexedSessionMetas,
  getIndexedSessionProjection,
  deleteSessionByFilePath,
  replaceSession,
  getSessionRecord,
} from "./db/session-store";
export { getMessagesForPage, getMessagesForRange } from "./db/message-store";
export { listSessions } from "./db/list-store";
export { getStatsCounts, getTopCwds } from "./db/stats-store";
export {
  coverageEntriesForSession,
  coverageStatusForSelector,
  cleanupMismatchedMessagesForSelector,
  countSessionsForSelector,
  deleteSessionsForSelectorExceptFilePaths,
  listCoverageRecords,
  replaceCoverage,
} from "./db/coverage-store";
export {
  buildSourceFileMetaResolver,
  loadSourceFileMetaCache,
  upsertSourceFileMetaCache,
} from "./db/file-meta-cache";
export { selectorWhereSql } from "./db/sql";
