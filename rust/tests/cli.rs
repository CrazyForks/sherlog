use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::SystemTime;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn shlog() -> Command {
    Command::new(env!("CARGO_BIN_EXE_shlog"))
}

fn isolated(command: &mut Command, home: &Path) {
    command
        .env("HOME", home)
        .env_remove("SHLOG_DATA_DIR")
        .env_remove("CXS_DATA_DIR")
        .env_remove("XDG_STATE_HOME");
}

fn run_isolated(args: &[&str], home: &Path) -> Output {
    let mut command = shlog();
    command.args(args);
    isolated(&mut command, home);
    command.output().expect("shlog should run")
}

#[cfg(unix)]
#[derive(Debug, Eq, PartialEq)]
struct TreeSnapshotEntry {
    path: PathBuf,
    kind: &'static str,
    mode: u32,
    len: u64,
    modified: Option<SystemTime>,
    contents: Vec<u8>,
    link_target: Option<PathBuf>,
}

#[cfg(unix)]
fn tree_snapshot(root: &Path) -> Vec<TreeSnapshotEntry> {
    fn collect(root: &Path, path: &Path, entries: &mut Vec<TreeSnapshotEntry>) {
        let metadata = fs::symlink_metadata(path).unwrap();
        let file_type = metadata.file_type();
        let relative = path.strip_prefix(root).unwrap();
        entries.push(TreeSnapshotEntry {
            path: if relative.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                relative.to_path_buf()
            },
            kind: if file_type.is_symlink() {
                "symlink"
            } else if file_type.is_dir() {
                "directory"
            } else if file_type.is_file() {
                "file"
            } else {
                "other"
            },
            mode: metadata.permissions().mode() & 0o777,
            len: metadata.len(),
            modified: metadata.modified().ok(),
            contents: if file_type.is_file() {
                fs::read(path).unwrap()
            } else {
                Vec::new()
            },
            link_target: file_type.is_symlink().then(|| fs::read_link(path).unwrap()),
        });
        if file_type.is_dir() {
            let mut children = fs::read_dir(path)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect::<Vec<_>>();
            children.sort();
            for child in children {
                collect(root, &child, entries);
            }
        }
    }

    let mut entries = Vec::new();
    collect(root, root, &mut entries);
    entries
}

#[cfg(unix)]
struct RestoreDirectoryPermissions(PathBuf);

#[cfg(unix)]
impl Drop for RestoreDirectoryPermissions {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
    }
}

#[test]
fn help_lists_the_complete_command_surface() {
    let output = shlog().arg("--help").output().expect("shlog should run");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());

    let stdout = String::from_utf8(output.stdout).expect("help should be UTF-8");
    for command in [
        "status",
        "sync",
        "cold",
        "find",
        "read-range",
        "read-page",
        "list",
        "stats",
    ] {
        assert!(stdout.contains(command), "help omitted {command}");
    }
}

#[test]
fn version_matches_the_workspace_package_version() {
    let output = shlog().arg("--version").output().expect("shlog should run");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("version should be UTF-8"),
        format!("{}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn native_find_reports_actionable_missing_index_on_stdout() {
    let home = tempfile::tempdir().unwrap();
    let output = run_isolated(&["find", "needle", "--json"], home.path());
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());

    let payload: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_eq!(payload["error"]["code"], "index_unavailable");
    assert_eq!(
        payload["error"]["nextAction"]["commands"][0]["argv"],
        serde_json::json!(["shlog", "sync"])
    );
}

#[test]
fn parse_errors_remain_plaintext_stderr_even_when_json_is_requested() {
    let output = shlog()
        .args(["find", "--json"])
        .output()
        .expect("shlog should run");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"error: missing required argument 'query'\n");
}

#[test]
fn unsupported_source_json_uses_stdout_and_exit_one() {
    let home = tempfile::tempdir().unwrap();
    let output = run_isolated(&["stats", "--source", "future", "--json"], home.path());
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["error"]["code"], "unsupported_source");
    assert_eq!(payload["error"]["source"], "future");
}

#[test]
fn cold_json_add_list_remove_round_trip() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join("archived sessions");
    let db = home.path().join("custom-state/index.sqlite");
    fs::create_dir_all(&root).unwrap();
    let root_text = root.to_str().unwrap();
    let db_text = db.to_str().unwrap();

    let added = run_isolated(
        &[
            "cold", "add", "--root", root_text, "--source", "pi", "--db", db_text, "--json",
        ],
        home.path(),
    );
    assert!(added.status.success());
    assert!(added.stderr.is_empty());
    let payload: serde_json::Value = serde_json::from_slice(&added.stdout).unwrap();
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["entry"]["sourceId"], "pi");
    assert_eq!(payload["entry"]["root"], root_text);
    assert_eq!(
        payload["configPath"],
        db.parent()
            .unwrap()
            .join("cold-roots.json")
            .to_str()
            .unwrap()
    );

    let listed = run_isolated(
        &["cold", "list", "--source", "pi", "--db", db_text, "--json"],
        home.path(),
    );
    assert!(listed.status.success());
    let payload: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(payload["roots"].as_array().unwrap().len(), 1);
    assert_eq!(payload["roots"][0]["root"], root_text);

    let removed = run_isolated(
        &[
            "cold", "remove", "--root", root_text, "--source", "pi", "--db", db_text, "--json",
        ],
        home.path(),
    );
    assert!(removed.status.success());
    let payload: serde_json::Value = serde_json::from_slice(&removed.stdout).unwrap();
    assert_eq!(payload["removed"], true);
    assert_eq!(payload["root"], root_text);
    assert_eq!(payload["sourceId"], "pi");

    let removed_again = run_isolated(
        &[
            "cold", "remove", "--root", root_text, "--source", "pi", "--db", db_text, "--json",
        ],
        home.path(),
    );
    assert!(removed_again.status.success());
    let payload: serde_json::Value = serde_json::from_slice(&removed_again.stdout).unwrap();
    assert_eq!(payload["removed"], false);
}

#[test]
fn cold_text_and_invalid_root_channels_match_the_published_cli() {
    let home = tempfile::tempdir().unwrap();
    let db = home.path().join("state/index.sqlite");
    let db_text = db.to_str().unwrap();

    let listed = run_isolated(&["cold", "list", "--db", db_text], home.path());
    assert!(listed.status.success());
    assert!(listed.stderr.is_empty());
    let text = String::from_utf8(listed.stdout).unwrap();
    assert!(text.starts_with("no cold roots registered\n"));
    assert!(text.contains("config: "));

    let missing = home.path().join("missing");
    let invalid = run_isolated(
        &[
            "cold",
            "add",
            "--root",
            missing.to_str().unwrap(),
            "--db",
            db_text,
            "--json",
        ],
        home.path(),
    );
    assert_eq!(invalid.status.code(), Some(1));
    assert!(invalid.stderr.is_empty());
    let payload: serde_json::Value = serde_json::from_slice(&invalid.stdout).unwrap();
    assert_eq!(payload["error"]["code"], "invalid_cold_root");
    assert!(
        payload["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("cold root does not exist:")
    );
}

#[cfg(unix)]
#[test]
fn every_read_only_command_succeeds_without_writing_a_read_only_index_directory() {
    let home = tempfile::tempdir().unwrap();
    let raw_root = home.path().join("sessions");
    let id = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee";
    let raw = raw_root
        .join("2026/08/15")
        .join(format!("rollout-2026-08-15T00-00-00-{id}.jsonl"));
    fs::create_dir_all(raw.parent().unwrap()).unwrap();
    fs::write(
        &raw,
        format!(
            "{}\n{}\n{}\n",
            serde_json::json!({
                "timestamp": "2026-08-15T00:00:00Z",
                "type": "session_meta",
                "payload": {"id": id, "cwd": "/repo/read-only"},
            }),
            serde_json::json!({
                "timestamp": "2026-08-15T00:00:01Z",
                "type": "event_msg",
                "payload": {"type": "user_message", "message": "read only beacon"},
            }),
            serde_json::json!({
                "timestamp": "2026-08-15T00:00:02Z",
                "type": "event_msg",
                "payload": {"type": "agent_message", "message": "read only response"},
            })
        ),
    )
    .unwrap();

    let state = home.path().join("state");
    let db = state.join("index.sqlite");
    let db_text = db.to_str().unwrap();
    let raw_text = raw_root.to_str().unwrap();
    let synced = run_isolated(
        &["sync", "--root", raw_text, "--db", db_text, "--json"],
        home.path(),
    );
    assert!(
        synced.status.success(),
        "stderr={} stdout={}",
        String::from_utf8_lossy(&synced.stderr),
        String::from_utf8_lossy(&synced.stdout)
    );
    assert!(db.exists());
    for suffix in ["-journal", "-wal", "-shm"] {
        assert!(!PathBuf::from(format!("{}{suffix}", db.display())).exists());
    }

    fs::set_permissions(&db, fs::Permissions::from_mode(0o444)).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o555)).unwrap();
    let _restore_permissions = RestoreDirectoryPermissions(state.clone());
    let before = tree_snapshot(&state);
    let session_ref = format!("codex:{id}");
    let commands = [
        vec!["status", "--root", raw_text, "--db", db_text, "--json"],
        vec!["cold", "list", "--db", db_text, "--json"],
        vec!["find", "read only beacon", "--db", db_text, "--json"],
        vec![
            "read-range",
            session_ref.as_str(),
            "--query",
            "read only beacon",
            "--db",
            db_text,
            "--json",
        ],
        vec![
            "read-page",
            session_ref.as_str(),
            "--offset",
            "0",
            "--limit",
            "10",
            "--db",
            db_text,
            "--json",
        ],
        vec!["list", "--db", db_text, "--json"],
        vec!["stats", "--db", db_text, "--json"],
    ];
    for command in commands {
        let output = run_isolated(&command, home.path());
        assert!(
            output.status.success(),
            "{} failed: stderr={} stdout={}",
            command[0],
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
    }
    let after = tree_snapshot(&state);
    assert_eq!(after, before, "read-only commands changed index state");
}

#[test]
fn read_only_commands_never_migrate_legacy_state_but_default_sync_does() {
    let home = tempfile::tempdir().unwrap();
    let legacy = home.path().join(".local/state/cxs");
    let destination = home.path().join(".local/state/shlog");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("sentinel"), "legacy-state").unwrap();
    let raw_root = home.path().join(".codex/sessions");
    let id = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
    let raw = raw_root
        .join("2026/08/15")
        .join(format!("rollout-2026-08-15T00-00-00-{id}.jsonl"));
    fs::create_dir_all(raw.parent().unwrap()).unwrap();
    fs::write(
        &raw,
        format!(
            "{}\n{}\n",
            serde_json::json!({
                "timestamp": "2026-08-15T00:00:00Z",
                "type": "session_meta",
                "payload": {"id": id, "cwd": "/repo"},
            }),
            serde_json::json!({
                "timestamp": "2026-08-15T00:00:01Z",
                "type": "event_msg",
                "payload": {"type": "user_message", "message": "migration evidence"},
            })
        ),
    )
    .unwrap();

    let version = run_isolated(&["--version"], home.path());
    assert!(version.status.success());
    assert!(legacy.exists());
    assert!(!destination.exists());

    let status = run_isolated(&["status", "--json"], home.path());
    assert!(status.status.success());
    assert!(legacy.exists());
    assert!(!destination.exists());

    let find = run_isolated(&["find", "migration", "--json"], home.path());
    assert_eq!(find.status.code(), Some(1));
    assert!(legacy.exists());
    assert!(!destination.exists());

    let listed = run_isolated(&["cold", "list", "--json"], home.path());
    assert!(listed.status.success());
    assert!(legacy.exists());
    assert!(!destination.exists());

    let synced = run_isolated(
        &["sync", "--root", raw_root.to_str().unwrap(), "--json"],
        home.path(),
    );
    assert!(
        synced.status.success(),
        "stderr={} stdout={}",
        String::from_utf8_lossy(&synced.stderr),
        String::from_utf8_lossy(&synced.stdout)
    );
    assert!(!legacy.exists());
    assert_eq!(
        fs::read_to_string(destination.join("sentinel")).unwrap(),
        "legacy-state"
    );
    assert!(destination.join("index.sqlite").exists());
}

#[test]
fn default_writer_fails_closed_when_legacy_and_destination_both_exist() {
    let home = tempfile::tempdir().unwrap();
    let legacy = home.path().join(".local/state/cxs");
    let destination = home.path().join(".local/state/shlog");
    fs::create_dir_all(&legacy).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(legacy.join("legacy-sentinel"), "legacy").unwrap();
    fs::write(destination.join("destination-sentinel"), "destination").unwrap();

    let synced = run_isolated(&["sync", "--json"], home.path());
    assert_eq!(synced.status.code(), Some(1));
    assert!(synced.stderr.is_empty());
    let payload: serde_json::Value = serde_json::from_slice(&synced.stdout).unwrap();
    assert_eq!(payload["error"]["code"], "index_error");
    assert!(
        payload["error"]["message"]
            .as_str()
            .unwrap()
            .contains("both exist")
    );
    assert_eq!(
        fs::read_to_string(legacy.join("legacy-sentinel")).unwrap(),
        "legacy"
    );
    assert_eq!(
        fs::read_to_string(destination.join("destination-sentinel")).unwrap(),
        "destination"
    );
    assert!(!destination.join("index.sqlite").exists());
}
