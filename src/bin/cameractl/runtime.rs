//! Terminal detection and the session options shared by every command.

use std::ffi::OsString;

use std::time::Duration;

use sony_ptp3::{ConnectOptions, FunctionMode};

use crate::cli::{Format, FunctionModeArg, GlobalOpts};
use crate::error::CliError;
use crate::parse;

#[derive(Debug)]
pub struct Runtime {
    pub format: Format,
    pub color: bool,
    pub verbose: bool,
    pub quiet: bool,
    pub stdout_is_tty: bool,
    pub options: ConnectOptions,
}

impl Runtime {
    pub fn from(global: &GlobalOpts, stdout_is_tty: bool) -> Result<Self, CliError> {
        if global.json {
            if let Some(format) = global.format {
                if format != Format::Json {
                    return Err(CliError::usage(
                        "--json cannot be combined with a different --format",
                        "Pass --json or --format json.",
                    ));
                }
            }
        }
        let format = if global.json {
            Format::Json
        } else if let Some(format) = global.format {
            format
        } else if stdout_is_tty {
            Format::Table
        } else {
            Format::Json
        };
        let color = stdout_is_tty && !global.no_color && !no_color_env() && !term_dumb();
        let _ = global.color;
        Ok(Self {
            format,
            color,
            verbose: global.verbose,
            quiet: global.quiet,
            stdout_is_tty,
            options: options(global)?,
        })
    }
}

fn options(global: &GlobalOpts) -> Result<ConnectOptions, CliError> {
    let mut options = ConnectOptions::default();
    if let Some(raw) = &global.connect_timeout {
        options.timeout = parse::duration(raw, Duration::from_millis(1), Duration::from_secs(120))?;
    }
    options.reset_device = global.reset_device;
    // Neither flag means the library default: release Image Capture on macOS.
    if global.no_release_system_client {
        options.release_system_client = false;
    } else if global.release_system_client {
        options.release_system_client = true;
    }
    options.function_mode = global.function_mode.map(|mode| match mode {
        FunctionModeArg::RemoteControl => FunctionMode::RemoteControl,
        FunctionModeArg::ContentTransfer => FunctionMode::ContentTransfer,
        FunctionModeArg::RemoteWithTransfer => FunctionMode::RemoteWithTransfer,
    });
    Ok(options)
}

#[derive(Debug, Default)]
pub struct Log {
    pub stderr: String,
}

impl Log {
    pub fn note(&mut self, runtime: &Runtime, message: &str) {
        if runtime.verbose {
            self.line(message);
        }
    }

    pub fn warn(&mut self, runtime: &Runtime, message: &str) {
        if !runtime.quiet {
            self.line(&format!("warning: {message}"));
        }
    }

    fn line(&mut self, message: &str) {
        self.stderr.push_str(message);
        if !message.ends_with('\n') {
            self.stderr.push('\n');
        }
    }
}

pub fn session_mode(mode: Option<FunctionMode>) -> &'static str {
    match mode {
        None => "standard",
        Some(FunctionMode::RemoteControl) => "remote-control",
        Some(FunctionMode::ContentTransfer) => "content-transfer",
        Some(FunctionMode::RemoteWithTransfer) => "remote-with-transfer",
    }
}

pub fn color_enabled(args: &[OsString], tty: bool) -> bool {
    if !tty || args.iter().any(|arg| arg == "--no-color") || no_color_env() || term_dumb() {
        return false;
    }
    true
}

pub fn wants_json_errors(args: &[OsString], tty: bool) -> bool {
    if !tty {
        return true;
    }
    let mut format_flag = false;
    for arg in args {
        let text = arg.to_string_lossy();
        if text == "--json"
            || text == "--format=json"
            || text == "--format=csv"
            || text == "--format=ndjson"
        {
            return true;
        }
        if format_flag && matches!(text.as_ref(), "json" | "csv" | "ndjson") {
            return true;
        }
        format_flag = text == "--format";
    }
    false
}

fn no_color_env() -> bool {
    std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty())
}

fn term_dumb() -> bool {
    std::env::var("TERM").ok().as_deref() == Some("dumb")
}
