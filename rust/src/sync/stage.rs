use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::sources::{ProjectionOutcome, SourceFile};

static STAGE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A disk-backed, disposable projection stream. Keeping only one serialized
/// session in memory bounds ingest memory by the largest session rather than
/// by the corpus. The file is never a cross-process commit plan and is always
/// removed on drop.
pub(crate) struct ProjectionStage {
    path: PathBuf,
    writer: Option<BufWriter<File>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StagedProjection {
    pub file: SourceFile,
    pub outcome: ProjectionOutcome,
}

impl ProjectionStage {
    pub(crate) fn create(parent: &Path) -> io::Result<Self> {
        fs::create_dir_all(parent)?;
        for _ in 0..128 {
            let sequence = STAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!(
                ".shlog-sync-stage-{}-{sequence}.jsonl",
                std::process::id()
            ));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            match options.open(&path) {
                Ok(file) => {
                    return Ok(Self {
                        path,
                        writer: Some(BufWriter::new(file)),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate a unique sync staging file",
        ))
    }

    pub(crate) fn push(&mut self, record: &StagedProjection) -> io::Result<()> {
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| io::Error::other("projection stage is sealed"))?;
        serde_json::to_writer(&mut *writer, record).map_err(io::Error::other)?;
        writer.write_all(b"\n")
    }

    pub(crate) fn seal(&mut self) -> io::Result<()> {
        let Some(mut writer) = self.writer.take() else {
            return Ok(());
        };
        writer.flush()?;
        writer.get_ref().sync_all()
    }

    pub(crate) fn read_all(
        &self,
        mut consume: impl FnMut(StagedProjection) -> io::Result<()>,
    ) -> io::Result<()> {
        if self.writer.is_some() {
            return Err(io::Error::other(
                "projection stage must be sealed before reading",
            ));
        }
        let reader = BufReader::new(File::open(&self.path)?);
        for line in reader.lines() {
            let line = line?;
            if line.is_empty() {
                continue;
            }
            let record = serde_json::from_str(&line).map_err(io::Error::other)?;
            consume(record)?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ProjectionStage {
    fn drop(&mut self) {
        self.writer.take();
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use crate::identity::SourceId;
    use crate::sources::{
        EmptyProjection, FileIdentity, FileStamp, ProjectionCheckpoint, ReadProof,
    };
    use tempfile::tempdir;

    use super::*;

    fn record(path: &Path) -> StagedProjection {
        let identity = FileIdentity::PathDigest {
            digest: "identity".to_owned(),
        };
        let proof = ReadProof {
            requested_limit: 0,
            effective_limit: 0,
            byte_count: 0,
            safe_offset: 0,
            content_fingerprint: "empty".to_owned(),
            safe_prefix_fingerprint: "empty".to_owned(),
            opened: FileStamp {
                mtime_ns: 0,
                size: 0,
                identity: identity.clone(),
            },
            completed: FileStamp {
                mtime_ns: 0,
                size: 0,
                identity: identity.clone(),
            },
        };
        StagedProjection {
            file: SourceFile {
                source_id: SourceId::Codex,
                file_path: path.to_path_buf(),
                path_date: None,
                cwd: String::new(),
                mtime_ms: 0.0,
                mtime_ns: 0,
                size: 0,
                identity: identity.clone(),
                accepted_fingerprint: String::new(),
            },
            outcome: ProjectionOutcome::Skipped(EmptyProjection {
                read_proof: proof,
                checkpoint: ProjectionCheckpoint {
                    source_id: SourceId::Codex,
                    file_identity: identity,
                    indexed_bytes: 0,
                    prefix_digest: "empty".to_owned(),
                    next_seq: 0,
                    reducer_state: "{}".to_owned(),
                },
            }),
        }
    }

    #[test]
    fn stage_can_be_replayed_more_than_once_and_is_deleted_on_drop() {
        let temp = tempdir().unwrap();
        let mut stage = ProjectionStage::create(temp.path()).unwrap();
        let expected = record(&temp.path().join("session.jsonl"));
        stage.push(&expected).unwrap();
        stage.seal().unwrap();
        let path = stage.path().to_path_buf();

        for _ in 0..2 {
            let mut actual = Vec::new();
            stage
                .read_all(|record| {
                    actual.push(record);
                    Ok(())
                })
                .unwrap();
            assert_eq!(actual, vec![expected.clone()]);
        }
        drop(stage);
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn stage_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempdir().unwrap();
        let stage = ProjectionStage::create(temp.path()).unwrap();
        let mode = fs::metadata(stage.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
