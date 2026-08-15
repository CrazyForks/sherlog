use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::identity::SourceId;
use crate::sources::{FileStamp, ProjectionMode, ReadProof, SourceFile};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotTransition {
    Stable,
    AppendOnly,
    Deferred,
    Rejected,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TransitionAssessment {
    pub kind: SnapshotTransition,
    pub deferred_paths: BTreeSet<PathBuf>,
    pub reason: Option<&'static str>,
}

#[derive(Clone, Debug)]
pub(crate) struct ProjectionProof {
    pub path: PathBuf,
    pub read: ReadProof,
    pub mode: ProjectionMode,
    pub was_indexed: bool,
    /// Result of hashing the current file through read.safe_offset and
    /// comparing it with the projection checkpoint's safe-prefix digest.
    pub current_prefix_matches: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct PersistedPrefixProof {
    pub path: PathBuf,
    pub current_prefix_matches: bool,
}

pub(crate) fn classify_transition(
    source: SourceId,
    before_files: &[SourceFile],
    before_file_set_fingerprint: &str,
    after_files: &[SourceFile],
    after_file_set_fingerprint: &str,
    projected: &[ProjectionProof],
    persisted: &[PersistedPrefixProof],
) -> TransitionAssessment {
    let before = file_map(before_files);
    let after = file_map(after_files);
    if before_file_set_fingerprint != after_file_set_fingerprint || before.keys().ne(after.keys()) {
        return rejected("source_file_set_changed");
    }

    let projected: BTreeMap<&PathBuf, &ProjectionProof> =
        projected.iter().map(|proof| (&proof.path, proof)).collect();
    let persisted: BTreeMap<&PathBuf, &PersistedPrefixProof> =
        persisted.iter().map(|proof| (&proof.path, proof)).collect();
    let mut changed = false;
    let mut deferred_paths = BTreeSet::new();

    for (path, before_file) in &before {
        let after_file = after[path];
        if same_stamp(before_file, after_file) {
            continue;
        }
        changed = true;
        if source != SourceId::Codex {
            return rejected("non_codex_source_changed");
        }
        if before_file.identity != after_file.identity || after_file.size <= before_file.size {
            return rejected("source_not_append_only");
        }

        if let Some(proof) = projected.get(path) {
            if !proof.current_prefix_matches
                || proof.read.effective_limit != before_file.size
                || proof.read.byte_count != before_file.size
                || proof.read.safe_offset > before_file.size
                || proof.read.opened.identity != before_file.identity
                || proof.read.completed.identity != before_file.identity
            {
                return rejected("projected_prefix_not_proven");
            }

            // A newly discovered file that had already changed before its
            // bounded open has no prior projection/cursor to anchor it. Keep
            // stable work but defer this file to the next explicit sync.
            if !proof.was_indexed && proof.read.opened != stamp(before_file) {
                deferred_paths.insert(path.clone());
                continue;
            }

            // Delta is the preferred active-file path. Full replacement is
            // still safe for an existing file because the current prefix was
            // re-hashed and the whole replacement is atomic.
            let _ = proof.mode;
            continue;
        }

        if !persisted
            .get(path)
            .is_some_and(|proof| proof.current_prefix_matches)
        {
            deferred_paths.insert(path.clone());
        }
    }

    if !changed {
        TransitionAssessment {
            kind: SnapshotTransition::Stable,
            deferred_paths,
            reason: None,
        }
    } else if deferred_paths.is_empty() {
        TransitionAssessment {
            kind: SnapshotTransition::AppendOnly,
            deferred_paths,
            reason: Some("source_content_changed"),
        }
    } else {
        TransitionAssessment {
            kind: SnapshotTransition::Deferred,
            deferred_paths,
            reason: Some("active_source_deferred"),
        }
    }
}

fn rejected(reason: &'static str) -> TransitionAssessment {
    TransitionAssessment {
        kind: SnapshotTransition::Rejected,
        deferred_paths: BTreeSet::new(),
        reason: Some(reason),
    }
}

fn file_map(files: &[SourceFile]) -> BTreeMap<PathBuf, &SourceFile> {
    files
        .iter()
        .map(|file| (file.file_path.clone(), file))
        .collect()
}

fn same_stamp(left: &SourceFile, right: &SourceFile) -> bool {
    left.mtime_ns == right.mtime_ns
        && left.size == right.size
        && left.identity == right.identity
        && left.path_date == right.path_date
        && left.cwd == right.cwd
        && left.accepted_fingerprint == right.accepted_fingerprint
}

fn stamp(file: &SourceFile) -> FileStamp {
    FileStamp {
        mtime_ns: file.mtime_ns,
        size: file.size,
        identity: file.identity.clone(),
    }
}

#[cfg(test)]
mod tests {
    use crate::identity::SourceId;
    use crate::sources::FileIdentity;

    use super::*;

    fn file(path: &str, size: u64, mtime_ns: i128) -> SourceFile {
        SourceFile {
            source_id: SourceId::Codex,
            file_path: PathBuf::from(path),
            path_date: Some("2026-08-15".to_owned()),
            cwd: "/work".to_owned(),
            mtime_ms: mtime_ns as f64 / 1_000_000.0,
            mtime_ns,
            size,
            identity: FileIdentity::PathDigest {
                digest: path.to_owned(),
            },
            accepted_fingerprint: String::new(),
        }
    }

    fn projection(before: &SourceFile, was_indexed: bool, prefix: bool) -> ProjectionProof {
        let stamp = stamp(before);
        ProjectionProof {
            path: before.file_path.clone(),
            read: ReadProof {
                requested_limit: before.size,
                effective_limit: before.size,
                byte_count: before.size,
                safe_offset: before.size,
                content_fingerprint: "content".to_owned(),
                safe_prefix_fingerprint: "prefix".to_owned(),
                opened: stamp.clone(),
                completed: stamp,
            },
            mode: ProjectionMode::Full,
            was_indexed,
            current_prefix_matches: prefix,
        }
    }

    #[test]
    fn accepts_proven_codex_append_and_marks_the_snapshot_soft_stale() {
        let before = file("/sessions/a.jsonl", 100, 10);
        let after = file("/sessions/a.jsonl", 120, 20);
        let result = classify_transition(
            SourceId::Codex,
            std::slice::from_ref(&before),
            "same-set",
            &[after],
            "same-set",
            &[projection(&before, true, true)],
            &[],
        );
        assert_eq!(result.kind, SnapshotTransition::AppendOnly);
        assert_eq!(result.reason, Some("source_content_changed"));
    }

    #[test]
    fn rejects_truncate_rewrite_and_file_set_change() {
        let before = file("/sessions/a.jsonl", 100, 10);
        let truncated = file("/sessions/a.jsonl", 80, 20);
        assert_eq!(
            classify_transition(
                SourceId::Codex,
                std::slice::from_ref(&before),
                "set",
                &[truncated],
                "set",
                &[],
                &[],
            )
            .kind,
            SnapshotTransition::Rejected
        );

        let grown = file("/sessions/a.jsonl", 120, 20);
        assert_eq!(
            classify_transition(
                SourceId::Codex,
                std::slice::from_ref(&before),
                "set",
                &[grown],
                "set",
                &[projection(&before, true, false)],
                &[],
            )
            .kind,
            SnapshotTransition::Rejected
        );

        assert_eq!(
            classify_transition(
                SourceId::Codex,
                &[before],
                "old-set",
                &[],
                "new-set",
                &[],
                &[],
            )
            .kind,
            SnapshotTransition::Rejected
        );
    }

    #[test]
    fn defers_a_new_file_that_changed_before_its_bounded_open() {
        let before = file("/sessions/new.jsonl", 100, 10);
        let after = file("/sessions/new.jsonl", 120, 20);
        let mut proof = projection(&before, false, true);
        proof.read.opened.size = 120;
        proof.read.opened.mtime_ns = 20;
        proof.read.completed = proof.read.opened.clone();
        let result = classify_transition(
            SourceId::Codex,
            std::slice::from_ref(&before),
            "set",
            &[after],
            "set",
            &[proof],
            &[],
        );
        assert_eq!(result.kind, SnapshotTransition::Deferred);
        assert!(result.deferred_paths.contains(&before.file_path));
    }

    #[test]
    fn an_unchanged_indexed_file_can_prove_a_mid_sync_append_from_its_cursor() {
        let before = file("/sessions/existing.jsonl", 100, 10);
        let after = file("/sessions/existing.jsonl", 110, 20);
        let result = classify_transition(
            SourceId::Codex,
            std::slice::from_ref(&before),
            "set",
            &[after],
            "set",
            &[],
            &[PersistedPrefixProof {
                path: before.file_path.clone(),
                current_prefix_matches: true,
            }],
        );
        assert_eq!(result.kind, SnapshotTransition::AppendOnly);
    }
}
