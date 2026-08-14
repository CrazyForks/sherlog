export class IndexUnavailableError extends Error {
  constructor(public readonly dbPath: string) {
    super(`index not found: ${dbPath}`);
    this.name = "IndexUnavailableError";
  }
}

export class IndexSchemaUpgradeRequiredError extends Error {
  constructor(
    public readonly dbPath: string,
    public readonly missingColumns: string[],
  ) {
    super(`index schema is too old for source-aware read commands: ${dbPath}`);
    this.name = "IndexSchemaUpgradeRequiredError";
  }
}
