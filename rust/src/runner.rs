//! CLI composition, typed error routing, and injectable command services.

use std::ffi::OsString;
use std::io::Write;

use clap::Parser;
use clap::error::ErrorKind;

use crate::app::NativeAppServices;
use crate::cli::{
    Cli, ColdArgs, ColdCommand, Command, FindArgs, ListArgs, ReadPageArgs, ReadRangeArgs,
    StatsArgs, StatusArgs, SyncArgs,
};
use crate::error::{AppError, EXIT_FAILURE, EXIT_SUCCESS};
use crate::identity::SourceId;

pub trait AppServices {
    /// Called once after successful CLI/source parsing and before dispatch.
    fn prepare(&mut self) -> Result<(), AppError> {
        Ok(())
    }

    fn status(
        &mut self,
        _args: &StatusArgs,
        _stdout: &mut dyn Write,
        _stderr: &mut dyn Write,
    ) -> Result<(), AppError> {
        Err(AppError::unsupported("status"))
    }

    fn sync(
        &mut self,
        _args: &SyncArgs,
        _stdout: &mut dyn Write,
        _stderr: &mut dyn Write,
    ) -> Result<(), AppError> {
        Err(AppError::unsupported("sync"))
    }

    fn cold(
        &mut self,
        _args: &ColdArgs,
        _stdout: &mut dyn Write,
        _stderr: &mut dyn Write,
    ) -> Result<(), AppError> {
        Err(AppError::unsupported("cold"))
    }

    fn find(
        &mut self,
        _args: &FindArgs,
        _stdout: &mut dyn Write,
        _stderr: &mut dyn Write,
    ) -> Result<(), AppError> {
        Err(AppError::unsupported("find"))
    }

    fn read_range(
        &mut self,
        _args: &ReadRangeArgs,
        _stdout: &mut dyn Write,
        _stderr: &mut dyn Write,
    ) -> Result<(), AppError> {
        Err(AppError::unsupported("read-range"))
    }

    fn read_page(
        &mut self,
        _args: &ReadPageArgs,
        _stdout: &mut dyn Write,
        _stderr: &mut dyn Write,
    ) -> Result<(), AppError> {
        Err(AppError::unsupported("read-page"))
    }

    fn list(
        &mut self,
        _args: &ListArgs,
        _stdout: &mut dyn Write,
        _stderr: &mut dyn Write,
    ) -> Result<(), AppError> {
        Err(AppError::unsupported("list"))
    }

    fn stats(
        &mut self,
        _args: &StatsArgs,
        _stdout: &mut dyn Write,
        _stderr: &mut dyn Write,
    ) -> Result<(), AppError> {
        Err(AppError::unsupported("stats"))
    }
}

pub fn run_from<I, T>(args: I, stdout: &mut dyn Write, stderr: &mut dyn Write) -> u8
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<OsString>>();
    if is_bare_version_request(&args) {
        return write_line(stdout, env!("CARGO_PKG_VERSION"));
    }
    let mut services = NativeAppServices::from_current_process();
    run_collected(&args, &mut services, stdout, stderr)
}

pub fn run_from_with_services<I, T, S>(
    args: I,
    services: &mut S,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
    S: AppServices + ?Sized,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<OsString>>();
    run_collected(&args, services, stdout, stderr)
}

pub fn dispatch_command<S: AppServices + ?Sized>(
    services: &mut S,
    command: &Command,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), AppError> {
    match command {
        Command::Status(args) => services.status(args, stdout, stderr),
        Command::Sync(args) => services.sync(args, stdout, stderr),
        Command::Cold(args) => services.cold(args, stdout, stderr),
        Command::Find(args) => services.find(args, stdout, stderr),
        Command::ReadRange(args) => services.read_range(args, stdout, stderr),
        Command::ReadPage(args) => services.read_page(args, stdout, stderr),
        Command::List(args) => services.list(args, stdout, stderr),
        Command::Stats(args) => services.stats(args, stdout, stderr),
    }
}

pub fn parse_public_source(value: &str) -> Result<SourceId, AppError> {
    let value = value.trim();
    value
        .parse::<SourceId>()
        .map_err(|_| AppError::unsupported_source(value))
}

fn run_collected<S: AppServices + ?Sized>(
    args: &[OsString],
    services: &mut S,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    if is_bare_version_request(args) {
        return write_line(stdout, env!("CARGO_PKG_VERSION"));
    }

    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            return match write!(stdout, "{error}") {
                Ok(()) => EXIT_SUCCESS,
                Err(write_error) => {
                    emit_error(&AppError::output(write_error), false, stdout, stderr)
                }
            };
        }
        Err(error) => return emit_parse_error(&error, args, stderr),
    };

    if let Err(error) = validate_command_sources(&cli.command) {
        return emit_error(&error, cli.json_output(), stdout, stderr);
    }
    if let Err(error) = services.prepare() {
        return emit_error(&error, cli.json_output(), stdout, stderr);
    }
    match dispatch_command(services, &cli.command, stdout, stderr) {
        Ok(()) => EXIT_SUCCESS,
        Err(error) => emit_error(&error, cli.json_output(), stdout, stderr),
    }
}

fn emit_parse_error(error: &clap::Error, args: &[OsString], stderr: &mut dyn Write) -> u8 {
    let compatibility_message = if error.kind() == ErrorKind::MissingRequiredArgument
        && args.get(1).and_then(|arg| arg.to_str()) == Some("find")
    {
        Some("error: missing required argument 'query'\n")
    } else {
        None
    };
    let result = match compatibility_message {
        Some(message) => write!(stderr, "{message}"),
        None => write!(stderr, "{error}"),
    };
    let _ = result;
    EXIT_FAILURE
}

fn validate_command_sources(command: &Command) -> Result<(), AppError> {
    match command {
        Command::Status(args) => {
            validate_optional_source(args.source.as_deref())?;
            validate_selector_source(args.selector.as_deref())
        }
        Command::Sync(args) => {
            validate_optional_source(args.source.as_deref())?;
            validate_selector_source(args.selector.as_deref())
        }
        Command::Cold(args) => match &args.command {
            ColdCommand::Add(args) => parse_public_source(&args.source).map(|_| ()),
            ColdCommand::List(args) => validate_optional_source(args.source.as_deref()),
            ColdCommand::Remove(args) => parse_public_source(&args.source).map(|_| ()),
        },
        Command::Find(args) => {
            if let Some(source) = args.source.as_deref()
                && source.trim() != "all"
            {
                parse_public_source(source)?;
            }
            validate_selector_source(args.selector.as_deref())
        }
        Command::ReadRange(args) => validate_read_source(args.source.as_deref(), &args.session_ref),
        Command::ReadPage(args) => validate_read_source(args.source.as_deref(), &args.session_ref),
        Command::List(args) => {
            validate_optional_source(args.source.as_deref())?;
            validate_selector_source(args.selector.as_deref())
        }
        Command::Stats(args) => validate_optional_source(args.source.as_deref()),
    }
}

fn validate_optional_source(source: Option<&str>) -> Result<(), AppError> {
    source.map(parse_public_source).transpose().map(|_| ())
}

fn validate_read_source(source: Option<&str>, session_ref: &str) -> Result<(), AppError> {
    if let Some(source) = source {
        return parse_public_source(source).map(|_| ());
    }
    if let Some((prefix, _)) = session_ref.split_once(':')
        && !prefix.is_empty()
    {
        return parse_public_source(prefix).map(|_| ());
    }
    Ok(())
}

fn validate_selector_source(selector_json: Option<&str>) -> Result<(), AppError> {
    let Some(selector_json) = selector_json else {
        return Ok(());
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(selector_json) else {
        return Ok(());
    };
    let Some(source) = value.get("source").and_then(serde_json::Value::as_str) else {
        return Ok(());
    };
    parse_public_source(source).map(|_| ())
}

fn emit_error<'a>(
    error: &AppError,
    json: bool,
    stdout: &'a mut dyn Write,
    stderr: &'a mut dyn Write,
) -> u8 {
    if error.is_silent() {
        return error.exit_code();
    }
    let writer = if json && error.json_uses_stdout() {
        stdout
    } else {
        stderr
    };
    let result = if json {
        serde_json::to_writer_pretty(&mut *writer, &error.envelope())
            .map_err(AppError::output)
            .and_then(|()| writeln!(writer).map_err(AppError::output))
    } else if error.plain_message_is_unadorned() {
        writeln!(writer, "{}", error.message()).map_err(AppError::output)
    } else {
        writeln!(writer, "error[{}]: {}", error.code(), error.message()).map_err(AppError::output)
    };

    match result {
        Ok(()) => error.exit_code(),
        Err(_) => EXIT_FAILURE,
    }
}

fn write_line(writer: &mut dyn Write, value: &str) -> u8 {
    match writeln!(writer, "{value}") {
        Ok(()) => EXIT_SUCCESS,
        Err(_) => EXIT_FAILURE,
    }
}

fn is_bare_version_request(args: &[OsString]) -> bool {
    args.len() == 2 && matches!(args[1].to_str(), Some("--version" | "-V"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingServices {
        prepare_count: usize,
        status_count: usize,
    }

    impl AppServices for RecordingServices {
        fn prepare(&mut self) -> Result<(), AppError> {
            self.prepare_count += 1;
            Ok(())
        }

        fn status(
            &mut self,
            _args: &StatusArgs,
            stdout: &mut dyn Write,
            _stderr: &mut dyn Write,
        ) -> Result<(), AppError> {
            self.status_count += 1;
            writeln!(stdout, "injected status").map_err(AppError::output)
        }
    }

    #[test]
    fn injected_services_receive_typed_dispatch_after_prepare() {
        let mut services = RecordingServices::default();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code =
            run_from_with_services(["shlog", "status"], &mut services, &mut stdout, &mut stderr);

        assert_eq!(code, EXIT_SUCCESS);
        assert_eq!(services.prepare_count, 1);
        assert_eq!(services.status_count, 1);
        assert_eq!(stdout, b"injected status\n");
        assert!(stderr.is_empty());
    }

    #[test]
    fn version_does_not_prepare_injected_services() {
        let mut services = RecordingServices::default();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run_from_with_services(
            ["shlog", "--version"],
            &mut services,
            &mut stdout,
            &mut stderr,
        );

        assert_eq!(code, EXIT_SUCCESS);
        assert_eq!(services.prepare_count, 0);
        assert_eq!(services.status_count, 0);
        assert!(stderr.is_empty());
    }

    #[test]
    fn unsupported_json_still_uses_typed_stderr_error() {
        let mut services = RecordingServices::default();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run_from_with_services(
            ["shlog", "find", "needle", "--json"],
            &mut services,
            &mut stdout,
            &mut stderr,
        );

        assert_eq!(code, EXIT_FAILURE);
        assert!(stdout.is_empty());
        let payload: serde_json::Value = serde_json::from_slice(&stderr).unwrap();
        assert_eq!(payload["error"]["code"], "unsupported_operation");
        assert_eq!(payload["error"]["operation"], "find");
    }

    #[test]
    fn missing_find_query_keeps_the_published_plaintext_parse_contract() {
        let mut services = RecordingServices::default();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run_from_with_services(
            ["shlog", "find", "--json"],
            &mut services,
            &mut stdout,
            &mut stderr,
        );

        assert_eq!(code, EXIT_FAILURE);
        assert!(stdout.is_empty());
        assert_eq!(stderr, b"error: missing required argument 'query'\n");
        assert_eq!(services.prepare_count, 0);
    }

    #[test]
    fn unknown_argument_is_stable_plaintext_even_when_json_is_present() {
        let mut services = RecordingServices::default();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run_from_with_services(
            ["shlog", "stats", "--unknown", "--json"],
            &mut services,
            &mut stdout,
            &mut stderr,
        );

        assert_eq!(code, EXIT_FAILURE);
        assert!(stdout.is_empty());
        let stderr = String::from_utf8(stderr).unwrap();
        assert!(stderr.starts_with("error: unexpected argument '--unknown' found\n"));
        assert!(!stderr.starts_with('{'));
        assert_eq!(services.prepare_count, 0);
    }

    #[test]
    fn unsupported_source_json_uses_stdout_before_service_dispatch() {
        let mut services = RecordingServices::default();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run_from_with_services(
            ["shlog", "status", "--source", "future", "--json"],
            &mut services,
            &mut stdout,
            &mut stderr,
        );

        assert_eq!(code, EXIT_FAILURE);
        assert_eq!(services.prepare_count, 0);
        assert_eq!(services.status_count, 0);
        assert!(stderr.is_empty());
        let payload: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(payload["error"]["code"], "unsupported_source");
        assert_eq!(payload["error"]["source"], "future");
    }

    #[test]
    fn find_all_remains_a_valid_special_source() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut services = RecordingServices::default();
        let code = run_from_with_services(
            ["shlog", "find", "needle", "--source", "all", "--json"],
            &mut services,
            &mut stdout,
            &mut stderr,
        );
        assert_eq!(code, EXIT_FAILURE);
        assert!(stdout.is_empty());
        let payload: serde_json::Value = serde_json::from_slice(&stderr).unwrap();
        assert_eq!(payload["error"]["code"], "unsupported_operation");
    }
}
