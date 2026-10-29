//! Agent-facing Sony camera control.
//!
//! stdout is data. stderr is messages. Exit codes are stable: 0 success, 2 usage,
//! 3 not found, 4 permission, 5 conflict, 6 retry, 10 dry-run.

mod app;
mod cli;
mod error;
mod idempotency;
mod names;
mod page;
mod parse;
mod paths;
mod present;
mod read_cmd;
mod render;
mod runtime;
mod schema;
mod session;
mod values;
mod write_cmd;

use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().collect();
    let tty = io::IsTerminal::is_terminal(&io::stdout());
    let finished = app::execute(&args, tty);
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    let _ = stdout.write_all(finished.stdout.as_bytes());
    let _ = stderr.write_all(finished.stderr.as_bytes());
    ExitCode::from(u8::try_from(finished.code).unwrap_or(1))
}
