import { homedir } from "node:os";
import { resolve } from "node:path";
import type {
  ParseSessionResult,
  Selector,
  SourceFileMeta,
  SourceInventory,
  SourceSnapshot,
} from "../types";
import type { CollectFilesOptions, SessionSourceAdapter, SourceSnapshotOptions } from "./types";

/**
 * TypeScript differential oracle does not read zstd-compressed DSH sessions.
 * The adapter exists so the CLI surface, source selection, and JSON contracts
 * stay aligned with the native Rust `shlog`; native sync must be used to
 * populate a database with `dsh` rows. Read-only commands treat DSH as an
 * empty indexed source rather than crashing cross-source `find`.
 */
const EMPTY_INVENTORY: SourceInventory = {
  root: "",
  totalFiles: 0,
  pathDateRange: { from: null, to: null },
  cwdGroups: [],
};

export const dshSourceAdapter: SessionSourceAdapter = {
  id: "dsh",
  public: true,
  displayName: "DSH",
  defaultRoot() {
    return resolve(homedir(), ".dsh", "sessions");
  },
  resolveRoot(override?: string) {
    return resolve(override ?? this.defaultRoot());
  },
  async collectFiles(_root: string, _options?: CollectFilesOptions): Promise<SourceFileMeta[]> {
    return [];
  },
  async inventoryFromFiles(root: string, files: SourceFileMeta[]): Promise<SourceInventory> {
    return {
      root,
      totalFiles: files.length,
      pathDateRange: { from: null, to: null },
      cwdGroups: [],
    };
  },
  async snapshotFromFiles(selector: Selector, files: SourceFileMeta[]): Promise<SourceSnapshot> {
    return {
      selector,
      fingerprint: "",
      fileSetFingerprint: "",
      fileCount: files.length,
      files,
    };
  },
  async collectInventory(root: string): Promise<SourceInventory> {
    return { ...EMPTY_INVENTORY, root };
  },
  async collectSnapshot(selector: Selector, _options?: SourceSnapshotOptions): Promise<SourceSnapshot> {
    return {
      selector,
      fingerprint: "",
      fileSetFingerprint: "",
      fileCount: 0,
      files: [],
    };
  },
  async parseFile(_file: SourceFileMeta): Promise<ParseSessionResult> {
    return { kind: "skipped" };
  },
};
