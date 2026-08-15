use std::fs::{self, File, Metadata};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::{FileIdentity, FileStamp, FullProjectionReason, ReadProof, SourceError, SourceFile};

pub(crate) struct BoundedRead {
    pub proof: ReadProof,
    pub safe_prefix_digest: String,
    pub prefix_at_parse: Option<String>,
    pub callback_failure: Option<FullProjectionReason>,
}

/// Stream complete JSONL records from a stable byte snapshot. The callback is
/// never invoked for an unterminated tail, even if that tail happens to be
/// valid JSON; this keeps `safe_offset` suitable for a future append cursor.
pub(crate) fn read_bounded_lines(
    file: &SourceFile,
    requested_limit: u64,
    parse_from: u64,
    mut callback: impl FnMut(&Map<String, Value>, u64, u64) -> Result<(), FullProjectionReason>,
) -> Result<BoundedRead, SourceError> {
    let handle = File::open(&file.file_path).map_err(|source| SourceError::Io {
        operation: "open source file",
        path: file.file_path.clone(),
        source,
    })?;
    let opened_metadata = handle.metadata().map_err(|source| SourceError::Io {
        operation: "stat opened source file",
        path: file.file_path.clone(),
        source,
    })?;
    let opened = file_stamp(&file.file_path, &opened_metadata);
    let effective_limit = requested_limit.min(opened.size);

    let mut reader = BufReader::new(handle.take(effective_limit));
    let mut content_hasher = Sha256::new();
    let mut safe_hasher = Sha256::new();
    let mut offset = 0_u64;
    let mut safe_offset = 0_u64;
    let mut prefix_at_parse = (parse_from == 0).then(sha256_empty);
    let mut callback_failure = None;
    let mut buffer = Vec::new();

    loop {
        buffer.clear();
        let bytes_read =
            reader
                .read_until(b'\n', &mut buffer)
                .map_err(|source| SourceError::Io {
                    operation: "read source file",
                    path: file.file_path.clone(),
                    source,
                })?;
        if bytes_read == 0 {
            break;
        }

        content_hasher.update(&buffer);
        let line_start = offset;
        offset = offset.saturating_add(bytes_read as u64);
        let newline_terminated = buffer.last() == Some(&b'\n');
        if !newline_terminated {
            // A read limit or concurrent short read cut this record. It is
            // covered by content_fingerprint but not by the append-safe proof.
            continue;
        }

        safe_hasher.update(&buffer);
        safe_offset = offset;
        if safe_offset == parse_from {
            prefix_at_parse = Some(hex::encode(safe_hasher.clone().finalize()));
        }

        if line_start < parse_from || callback_failure.is_some() {
            continue;
        }
        if line_start != parse_from && prefix_at_parse.is_none() {
            callback_failure = Some(FullProjectionReason::CursorNotOnLineBoundary);
            continue;
        }

        let raw_end = line_start + content_len_without_newline(&buffer) as u64;
        let json = trim_ascii(&buffer[..content_len_without_newline(&buffer)]);
        if json.is_empty() {
            continue;
        }
        let Ok(Value::Object(record)) = serde_json::from_slice::<Value>(json) else {
            // Malformed and non-object lines are format drift, not searchable
            // content and not a whole-file parser failure.
            continue;
        };
        if let Err(reason) = callback(&record, line_start, raw_end) {
            callback_failure = Some(reason);
        }
    }

    let completed_metadata = fs::metadata(&file.file_path).map_err(|source| SourceError::Io {
        operation: "stat completed source file",
        path: file.file_path.clone(),
        source,
    })?;
    let completed = file_stamp(&file.file_path, &completed_metadata);
    let safe_prefix_digest = hex::encode(safe_hasher.finalize());
    let proof = ReadProof {
        requested_limit,
        effective_limit,
        byte_count: offset,
        safe_offset,
        content_fingerprint: hex::encode(content_hasher.finalize()),
        safe_prefix_fingerprint: safe_prefix_digest.clone(),
        opened,
        completed,
    };
    Ok(BoundedRead {
        proof,
        safe_prefix_digest,
        prefix_at_parse,
        callback_failure,
    })
}

/// Metadata scans share the streaming JSONL decoder but may accept a final
/// non-newline record only when the scan reached the captured EOF. Metadata is
/// never exposed as evidence and does not create an append cursor.
pub(crate) fn scan_json_records(
    path: &Path,
    byte_limit: Option<u64>,
    mut callback: impl FnMut(&Map<String, Value>) -> bool,
) -> Result<(), SourceError> {
    let handle = File::open(path).map_err(|source| SourceError::Io {
        operation: "open source metadata",
        path: path.to_path_buf(),
        source,
    })?;
    let size = handle
        .metadata()
        .map_err(|source| SourceError::Io {
            operation: "stat source metadata",
            path: path.to_path_buf(),
            source,
        })?
        .len();
    let limit = byte_limit.unwrap_or(size).min(size);
    let reached_eof = limit == size;
    let mut reader = BufReader::new(handle.take(limit));
    let mut buffer = Vec::new();

    loop {
        buffer.clear();
        let bytes_read =
            reader
                .read_until(b'\n', &mut buffer)
                .map_err(|source| SourceError::Io {
                    operation: "read source metadata",
                    path: path.to_path_buf(),
                    source,
                })?;
        if bytes_read == 0 {
            break;
        }
        let newline_terminated = buffer.last() == Some(&b'\n');
        if !newline_terminated && !reached_eof {
            break;
        }
        let content_len = if newline_terminated {
            content_len_without_newline(&buffer)
        } else {
            buffer.len()
        };
        let json = trim_ascii(&buffer[..content_len]);
        if json.is_empty() {
            continue;
        }
        let Ok(Value::Object(record)) = serde_json::from_slice::<Value>(json) else {
            continue;
        };
        if !callback(&record) {
            break;
        }
    }
    Ok(())
}

pub(crate) fn metadata_times(metadata: &Metadata) -> (i128, f64) {
    let nanoseconds = match metadata.modified().and_then(|value| {
        value
            .duration_since(UNIX_EPOCH)
            .map_err(std::io::Error::other)
    }) {
        Ok(duration) => {
            i128::from(duration.as_secs()) * 1_000_000_000 + i128::from(duration.subsec_nanos())
        }
        Err(_) => 0,
    };
    (nanoseconds, nanoseconds as f64 / 1_000_000.0)
}

pub(crate) fn file_identity(path: &Path, metadata: &Metadata) -> FileIdentity {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.dev() != 0 || metadata.ino() != 0 {
            return FileIdentity::Unix {
                device: metadata.dev(),
                inode: metadata.ino(),
            };
        }
    }

    let digest = Sha256::digest(path.to_string_lossy().as_bytes());
    FileIdentity::PathDigest {
        digest: hex::encode(digest),
    }
}

pub(crate) fn file_stamp(path: &Path, metadata: &Metadata) -> FileStamp {
    let (mtime_ns, _) = metadata_times(metadata);
    FileStamp {
        mtime_ns,
        size: metadata.len(),
        identity: file_identity(path, metadata),
    }
}

pub(crate) fn string(record: &Map<String, Value>, key: &str) -> String {
    record
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_owned()
}

pub(crate) fn raw_string(record: &Map<String, Value>, key: &str) -> String {
    record
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

pub(crate) fn object<'a>(
    record: &'a Map<String, Value>,
    key: &str,
) -> Option<&'a Map<String, Value>> {
    record.get(key).and_then(Value::as_object)
}

pub(crate) fn timestamp_date(timestamp: &str) -> Option<String> {
    let bytes = timestamp.as_bytes();
    let valid = bytes.len() >= 11
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[..10]
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit());
    valid.then(|| timestamp[..10].to_owned())
}

fn content_len_without_newline(buffer: &[u8]) -> usize {
    let mut end = buffer.len();
    if end > 0 && buffer[end - 1] == b'\n' {
        end -= 1;
    }
    if end > 0 && buffer[end - 1] == b'\r' {
        end -= 1;
    }
    end
}

fn trim_ascii(mut value: &[u8]) -> &[u8] {
    while value.first().is_some_and(u8::is_ascii_whitespace) {
        value = &value[1..];
    }
    while value.last().is_some_and(u8::is_ascii_whitespace) {
        value = &value[..value.len() - 1];
    }
    value
}

fn sha256_empty() -> String {
    hex::encode(Sha256::digest([]))
}

pub(crate) fn io_error(
    operation: &'static str,
    path: impl Into<PathBuf>,
    source: std::io::Error,
) -> SourceError {
    SourceError::Io {
        operation,
        path: path.into(),
        source,
    }
}
