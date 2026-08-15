use std::ffi::OsString;
use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args = std::env::args_os().collect::<Vec<OsString>>();
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    ExitCode::from(sherlog_cli::runner::run_from(
        args,
        &mut stdout,
        &mut stderr,
    ))
}
