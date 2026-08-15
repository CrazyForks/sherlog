use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use thiserror::Error;
use walkdir::WalkDir;

use crate::identity::SourceId;

#[derive(Debug, Error)]
pub(crate) enum ColdPresenceError {
    #[error("cold retention is not implemented for source {0}")]
    UnsupportedSource(SourceId),
    #[error("inspect cold root {path:?}: {message}")]
    Inspect { path: PathBuf, message: String },
    #[error("walk cold root {path:?}: {message}")]
    Walk { path: PathBuf, message: String },
}

/// Collect cold presence only. This deliberately does not decompress or parse
/// zstd data: a cold file protects the SQLite projection from explicit prune,
/// but is not a rehydration source.
pub(crate) fn collect_cold_native_ids(
    source: SourceId,
    roots: &[PathBuf],
) -> Result<BTreeSet<String>, ColdPresenceError> {
    if roots.is_empty() {
        return Ok(BTreeSet::new());
    }
    if source != SourceId::Codex {
        return Err(ColdPresenceError::UnsupportedSource(source));
    }

    let mut ids = BTreeSet::new();
    for root in roots {
        let metadata = match root.metadata() {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(ColdPresenceError::Inspect {
                    path: root.clone(),
                    message: error.to_string(),
                });
            }
        };
        if !metadata.is_dir() {
            return Err(ColdPresenceError::Inspect {
                path: root.clone(),
                message: "registered cold root is not a directory".to_owned(),
            });
        }
        for entry in WalkDir::new(root).follow_links(false) {
            let entry = entry.map_err(|error| ColdPresenceError::Walk {
                path: error
                    .path()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| root.clone()),
                message: error.to_string(),
            })?;
            if !entry.file_type().is_file() {
                continue;
            }
            if let Some(id) = codex_id_from_cold_path(entry.path()) {
                ids.insert(id);
            }
        }
    }
    Ok(ids)
}

fn codex_id_from_cold_path(path: &Path) -> Option<String> {
    let mut name = path.file_name()?.to_str()?;
    name = name.strip_suffix(".zst").unwrap_or(name);
    name = name.strip_suffix(".jsonl")?;
    let bytes = name.as_bytes();
    const UUID_LEN: usize = 36;
    if bytes.len() < UUID_LEN {
        return None;
    }
    for start in (0..=bytes.len() - UUID_LEN).rev() {
        let candidate = &bytes[start..start + UUID_LEN];
        if candidate.iter().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                *byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        }) {
            return Some(String::from_utf8_lossy(candidate).to_ascii_lowercase());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn recognizes_plain_and_zstd_codex_rollouts_without_reading_them() {
        let temp = tempdir().unwrap();
        let nested = temp.path().join("2026/08/15");
        fs::create_dir_all(&nested).unwrap();
        fs::write(
            nested
                .join("rollout-2026-08-15T00-00-00-AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA.jsonl.zst"),
            "not actually compressed",
        )
        .unwrap();
        fs::write(
            nested.join("rollout-2026-08-15T00-00-00-bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb.jsonl"),
            "not json",
        )
        .unwrap();
        fs::write(nested.join("unrelated.jsonl.zst"), "ignored").unwrap();

        assert_eq!(
            collect_cold_native_ids(SourceId::Codex, &[temp.path().to_path_buf()]).unwrap(),
            BTreeSet::from([
                "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_owned(),
                "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".to_owned(),
            ])
        );
    }

    #[test]
    fn refuses_destructive_non_codex_prune_when_cold_roots_are_supplied() {
        let error =
            collect_cold_native_ids(SourceId::ClaudeCode, &[PathBuf::from("/cold")]).unwrap_err();
        assert!(matches!(error, ColdPresenceError::UnsupportedSource(_)));
    }
}
