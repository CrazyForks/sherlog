/// Filesystem fence used while retiring legacy cold-root JSON state.
///
/// Both callbacks run while Sherlog's legacy-compatible writer lock is held.
/// `preflight` may validate permissions and prepare a durable recovery backup,
/// but must not make the legacy path unavailable. `publish` runs only after
/// the SQLite transaction is fully prepared and immediately before commit; it
/// must atomically make the legacy writer path unusable and be idempotent.
///
/// A successful publish is intentionally not rolled back if a later SQLite or
/// scratch-index publish fails. The caller must retain its recovery backup and,
/// while no v8 database exists, pass the recovered pending registrations into
/// the next operation. Reopening the legacy path would recreate a two-writer
/// window.
pub trait LegacyCutover {
    fn preflight(&mut self) -> Result<(), String>;
    fn publish(&mut self) -> Result<(), String>;

    /// Revalidate the durable backup and fence after SQLite commit (and, for
    /// first bootstrap, after the scratch database becomes active). This still
    /// runs under the writer lock and catches an old writer that opened the
    /// legacy inode before `publish` but flushed into it later. Failure means
    /// the database may already be committed; callers must surface that state
    /// rather than claiming rollback.
    fn complete(&mut self) -> Result<(), String> {
        Ok(())
    }
}

pub(crate) struct NoopLegacyCutover;

impl LegacyCutover for NoopLegacyCutover {
    fn preflight(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn publish(&mut self) -> Result<(), String> {
        Ok(())
    }
}
