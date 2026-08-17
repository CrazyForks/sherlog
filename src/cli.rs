use std::fmt;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::config::default_db_path;

#[derive(Debug, Parser)]
#[command(
    name = "shlog",
    version,
    about = "Sherlog progressive search CLI",
    subcommand_required = true,
    arg_required_else_help = true,
    color = clap::ColorChoice::Never
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    pub fn json_output(&self) -> bool {
        self.command.json_output()
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Return execution context, index metadata, and coverage proof.
    Status(StatusArgs),
    /// Scan local agent sessions and update the SQLite index.
    Sync(SyncArgs),
    /// Manage cold raw roots used by prune retention.
    Cold(ColdArgs),
    /// Search related sessions and return minimal evidence anchors.
    Find(FindArgs),
    /// Read local context around a message or query anchor.
    #[command(name = "read-range")]
    ReadRange(ReadRangeArgs),
    /// Read a session projection sequentially by page.
    #[command(name = "read-page")]
    ReadPage(ReadPageArgs),
    /// List indexed sessions without full-text search.
    List(ListArgs),
    /// Return index statistics.
    Stats(StatsArgs),
}

impl Command {
    pub fn operation(&self) -> &'static str {
        match self {
            Self::Status(_) => "status",
            Self::Sync(_) => "sync",
            Self::Cold(args) => args.command.operation(),
            Self::Find(_) => "find",
            Self::ReadRange(_) => "read-range",
            Self::ReadPage(_) => "read-page",
            Self::List(_) => "list",
            Self::Stats(_) => "stats",
        }
    }

    fn json_output(&self) -> bool {
        match self {
            Self::Status(args) => args.json,
            Self::Sync(args) => args.json,
            Self::Cold(args) => args.command.json_output(),
            Self::Find(args) => args.json,
            Self::ReadRange(args) => args.json,
            Self::ReadPage(args) => args.json,
            Self::List(args) => args.json,
            Self::Stats(args) => args.json,
        }
    }
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    /// Session source.
    #[arg(long)]
    pub source: Option<String>,
    /// Override the default sessions root and selector root.
    #[arg(long, value_name = "DIR")]
    pub root: Option<PathBuf>,
    /// Selector JSON used for a read-only coverage/freshness check.
    #[arg(long, value_name = "JSON")]
    pub selector: Option<String>,
    /// Check coverage for this cwd selector.
    #[arg(long, value_name = "PATH")]
    pub cwd: Option<PathBuf>,
    #[command(flatten)]
    pub database: DatabaseArg,
    /// Include historical coverage and cwd inventory details.
    #[arg(long)]
    pub inventory: bool,
    /// Emit JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct SyncArgs {
    /// Session source.
    #[arg(long)]
    pub source: Option<String>,
    /// Override the sessions root and selector root.
    #[arg(long, value_name = "DIR")]
    pub root: Option<PathBuf>,
    /// Structured selector JSON.
    #[arg(long, value_name = "JSON")]
    pub selector: Option<String>,
    /// Sync this cwd selector.
    #[arg(long, value_name = "PATH")]
    pub cwd: Option<PathBuf>,
    #[command(flatten)]
    pub database: DatabaseArg,
    /// Commit operations that succeeded even if some files fail.
    #[arg(long)]
    pub best_effort: bool,
    /// Remove rows absent from both the hot source and registered cold roots.
    #[arg(long)]
    pub prune: bool,
    /// Add a cold root for this sync only. May be repeated.
    #[arg(long, value_name = "DIR", action = clap::ArgAction::Append)]
    pub cold_root: Vec<PathBuf>,
    /// Emit JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ColdArgs {
    #[command(subcommand)]
    pub command: ColdCommand,
}

#[derive(Debug, Subcommand)]
pub enum ColdCommand {
    /// Register a cold root without parsing session bodies.
    Add(ColdAddArgs),
    /// List registered cold roots.
    List(ColdListArgs),
    /// Unregister a cold root without deleting files or index rows.
    Remove(ColdRemoveArgs),
}

impl ColdCommand {
    fn operation(&self) -> &'static str {
        match self {
            Self::Add(_) => "cold.add",
            Self::List(_) => "cold.list",
            Self::Remove(_) => "cold.remove",
        }
    }

    fn json_output(&self) -> bool {
        match self {
            Self::Add(args) => args.json,
            Self::List(args) => args.json,
            Self::Remove(args) => args.json,
        }
    }
}

#[derive(Debug, Args)]
pub struct ColdAddArgs {
    /// Cold sessions root.
    #[arg(long, value_name = "DIR")]
    pub root: PathBuf,
    /// Session source.
    #[arg(long, default_value = "codex")]
    pub source: String,
    #[command(flatten)]
    pub database: DatabaseArg,
    /// Emit JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ColdListArgs {
    /// Filter by session source.
    #[arg(long)]
    pub source: Option<String>,
    #[command(flatten)]
    pub database: DatabaseArg,
    /// Emit JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ColdRemoveArgs {
    /// Cold root to unregister.
    #[arg(long, value_name = "DIR")]
    pub root: PathBuf,
    /// Session source.
    #[arg(long, default_value = "codex")]
    pub source: String,
    #[command(flatten)]
    pub database: DatabaseArg,
    /// Emit JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct FindArgs {
    /// Query text.
    pub query: String,
    /// Session source filter. Defaults to all public sources.
    #[arg(long)]
    pub source: Option<String>,
    /// Maximum number of results.
    #[arg(
        short = 'n',
        long,
        default_value_t = 10,
        allow_hyphen_values = true,
        value_parser = find_limit
    )]
    pub limit: usize,
    /// Restrict search to this sessions root.
    #[arg(long, value_name = "DIR")]
    pub root: Option<PathBuf>,
    /// Structured selector JSON.
    #[arg(long, value_name = "JSON")]
    pub selector: Option<String>,
    /// Restrict search to this cwd selector.
    #[arg(long, value_name = "PATH")]
    pub cwd: Option<PathBuf>,
    /// Result ordering.
    #[arg(long, default_value_t = FindSort::Relevance, value_parser = parse_find_sort)]
    pub sort: FindSort,
    /// Exclude a session UUID or source-qualified session reference. May be repeated.
    #[arg(long, value_name = "SESSION", action = clap::ArgAction::Append)]
    pub exclude_session: Vec<String>,
    #[command(flatten)]
    pub database: DatabaseArg,
    /// Emit JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ReadRangeArgs {
    /// Bare Codex UUID or source-qualified session reference.
    #[arg(value_name = "SESSION_REF")]
    pub session_ref: String,
    /// Session source.
    #[arg(long)]
    pub source: Option<String>,
    /// Explicit message sequence anchor.
    #[arg(long, allow_hyphen_values = true, value_parser = parse_optional_int)]
    pub seq: Option<OptionalInt>,
    /// Locate an anchor within the session using this query.
    #[arg(long)]
    pub query: Option<String>,
    /// Messages before the anchor.
    #[arg(
        long,
        default_value_t = 2,
        allow_hyphen_values = true,
        value_parser = range_count
    )]
    pub before: usize,
    /// Messages after the anchor.
    #[arg(
        long,
        default_value_t = 2,
        allow_hyphen_values = true,
        value_parser = range_count
    )]
    pub after: usize,
    /// Maximum displayed characters per message; zero disables elision.
    #[arg(
        long,
        default_value_t = 800,
        allow_hyphen_values = true,
        value_parser = max_message_chars
    )]
    pub max_message_chars: usize,
    #[command(flatten)]
    pub database: DatabaseArg,
    /// Emit JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ReadPageArgs {
    /// Bare Codex UUID or source-qualified session reference.
    #[arg(value_name = "SESSION_REF")]
    pub session_ref: String,
    /// Session source.
    #[arg(long)]
    pub source: Option<String>,
    /// Starting message offset.
    #[arg(
        long,
        default_value_t = 0,
        allow_hyphen_values = true,
        value_parser = page_offset
    )]
    pub offset: usize,
    /// Page size.
    #[arg(
        long,
        default_value_t = 20,
        allow_hyphen_values = true,
        value_parser = page_limit
    )]
    pub limit: usize,
    /// Maximum displayed characters per message; zero disables elision.
    #[arg(
        long,
        default_value_t = 800,
        allow_hyphen_values = true,
        value_parser = max_message_chars
    )]
    pub max_message_chars: usize,
    #[command(flatten)]
    pub database: DatabaseArg,
    /// Emit JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Session source.
    #[arg(long)]
    pub source: Option<String>,
    /// Case-insensitive cwd substring filter.
    #[arg(long, value_name = "NEEDLE")]
    pub cwd: Option<String>,
    /// Include only sessions whose ended_at is at or after this value.
    #[arg(long, value_name = "ISO")]
    pub since: Option<String>,
    /// Restrict listing to this sessions root.
    #[arg(long, value_name = "DIR")]
    pub root: Option<PathBuf>,
    /// Structured selector JSON.
    #[arg(long, value_name = "JSON")]
    pub selector: Option<String>,
    /// Result ordering.
    #[arg(long, default_value_t = ListSort::Ended, value_parser = parse_list_sort)]
    pub sort: ListSort,
    /// Maximum number of sessions.
    #[arg(
        short = 'n',
        long,
        default_value_t = 20,
        allow_hyphen_values = true,
        value_parser = list_limit
    )]
    pub limit: usize,
    #[command(flatten)]
    pub database: DatabaseArg,
    /// Emit JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct StatsArgs {
    /// Session source.
    #[arg(long)]
    pub source: Option<String>,
    #[command(flatten)]
    pub database: DatabaseArg,
    /// Emit JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct DatabaseArg {
    /// Override the index database path.
    #[arg(long, value_name = "PATH", default_value_os_t = default_db_path())]
    pub db: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FindSort {
    Relevance,
    Ended,
    Started,
}

impl fmt::Display for FindSort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Relevance => "relevance",
            Self::Ended => "ended",
            Self::Started => "started",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ListSort {
    Ended,
    Started,
    Messages,
}

impl fmt::Display for ListSort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Ended => "ended",
            Self::Started => "started",
            Self::Messages => "messages",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OptionalInt(Option<i64>);

impl OptionalInt {
    pub const fn value(self) -> Option<i64> {
        self.0
    }
}

fn find_limit(value: &str) -> Result<usize, String> {
    Ok(positive_or(value, 10))
}

fn range_count(value: &str) -> Result<usize, String> {
    Ok(positive_or(value, 2))
}

fn page_limit(value: &str) -> Result<usize, String> {
    Ok(positive_or(value, 20))
}

fn list_limit(value: &str) -> Result<usize, String> {
    Ok(positive_or(value, 20))
}

fn page_offset(value: &str) -> Result<usize, String> {
    Ok(non_negative_or(value, 0))
}

fn max_message_chars(value: &str) -> Result<usize, String> {
    Ok(non_negative_or(value, 800))
}

fn parse_optional_int(value: &str) -> Result<OptionalInt, String> {
    Ok(OptionalInt(
        integer_prefix(value).and_then(|value| i64::try_from(value).ok()),
    ))
}

fn parse_find_sort(value: &str) -> Result<FindSort, String> {
    Ok(match value {
        "ended" => FindSort::Ended,
        "started" => FindSort::Started,
        _ => FindSort::Relevance,
    })
}

fn parse_list_sort(value: &str) -> Result<ListSort, String> {
    Ok(match value {
        "started" => ListSort::Started,
        "messages" => ListSort::Messages,
        _ => ListSort::Ended,
    })
}

fn positive_or(value: &str, fallback: usize) -> usize {
    integer_prefix(value)
        .filter(|value| *value > 0)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(fallback)
}

fn non_negative_or(value: &str, fallback: usize) -> usize {
    integer_prefix(value)
        .filter(|value| *value >= 0)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(fallback)
}

/// The decimal prefix accepted by JavaScript's `Number.parseInt(value, 10)`.
fn integer_prefix(value: &str) -> Option<i128> {
    let value = value.trim_start();
    let mut end = 0;
    let mut chars = value.char_indices();
    if let Some((_, '+' | '-')) = chars.next() {
        end = 1;
    } else {
        chars = value.char_indices();
    }
    let digit_start = end;
    for (index, character) in chars {
        if !character.is_ascii_digit() {
            break;
        }
        end = index + character.len_utf8();
    }
    if end == digit_start {
        return None;
    }
    value[..end].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_public_command() {
        let cases = [
            vec!["shlog", "status", "--json"],
            vec!["shlog", "sync", "--best-effort", "--prune", "--json"],
            vec!["shlog", "cold", "add", "--root", "/tmp/cold", "--json"],
            vec!["shlog", "cold", "list", "--json"],
            vec!["shlog", "cold", "remove", "--root", "/tmp/cold", "--json"],
            vec![
                "shlog",
                "find",
                "needle",
                "--source",
                "all",
                "--exclude-session",
                "codex:one",
                "--json",
            ],
            vec![
                "shlog",
                "read-range",
                "codex:one",
                "--query",
                "needle",
                "--json",
            ],
            vec!["shlog", "read-page", "codex:one", "--json"],
            vec!["shlog", "list", "--sort", "messages", "--json"],
            vec!["shlog", "stats", "--json"],
        ];

        for args in cases {
            let parsed = Cli::try_parse_from(args).expect("command should parse");
            assert!(parsed.json_output());
        }
    }

    #[test]
    fn uses_source_aware_operation_names() {
        let parsed = Cli::try_parse_from(["shlog", "cold", "remove", "--root", "/tmp/cold"])
            .expect("cold remove should parse");
        assert_eq!(parsed.command.operation(), "cold.remove");
    }

    #[test]
    fn accepts_unknown_sources_for_typed_dispatch_validation() {
        let parsed = Cli::try_parse_from(["shlog", "stats", "--source", "future", "--json"])
            .expect("source validation belongs to the runner");
        let Command::Stats(args) = parsed.command else {
            panic!("expected stats command");
        };
        assert_eq!(args.source.as_deref(), Some("future"));

        let parsed = Cli::try_parse_from(["shlog", "find", "needle", "--source", "all", "--json"])
            .expect("find all should parse");
        let Command::Find(args) = parsed.command else {
            panic!("expected find command");
        };
        assert_eq!(args.source.as_deref(), Some("all"));
    }

    #[test]
    fn numeric_options_match_parse_int_prefix_and_fallback_semantics() {
        let parsed = Cli::try_parse_from([
            "shlog",
            "read-range",
            "session",
            "--seq",
            "-7x",
            "--before",
            "0",
            "--after",
            "12items",
            "--max-message-chars",
            "-1",
        ])
        .unwrap();
        let Command::ReadRange(args) = parsed.command else {
            panic!("expected read-range command");
        };
        assert_eq!(args.seq.and_then(OptionalInt::value), Some(-7));
        assert_eq!(args.before, 2);
        assert_eq!(args.after, 12);
        assert_eq!(args.max_message_chars, 800);

        let parsed = Cli::try_parse_from([
            "shlog",
            "read-page",
            "session",
            "--offset",
            "invalid",
            "--limit",
            "0",
            "--max-message-chars",
            "0",
        ])
        .unwrap();
        let Command::ReadPage(args) = parsed.command else {
            panic!("expected read-page command");
        };
        assert_eq!(args.offset, 0);
        assert_eq!(args.limit, 20);
        assert_eq!(args.max_message_chars, 0);

        let parsed = Cli::try_parse_from(["shlog", "find", "needle", "--limit", "14x"]).unwrap();
        let Command::Find(args) = parsed.command else {
            panic!("expected find command");
        };
        assert_eq!(args.limit, 14);
    }

    #[test]
    fn unknown_sort_values_fall_back_instead_of_failing() {
        let parsed =
            Cli::try_parse_from(["shlog", "find", "needle", "--sort", "newest-ish"]).unwrap();
        let Command::Find(args) = parsed.command else {
            panic!("expected find command");
        };
        assert_eq!(args.sort, FindSort::Relevance);

        let parsed = Cli::try_parse_from(["shlog", "list", "--sort", "unknown"]).unwrap();
        let Command::List(args) = parsed.command else {
            panic!("expected list command");
        };
        assert_eq!(args.sort, ListSort::Ended);
    }
}
