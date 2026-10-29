//! Dispatch, exit codes, and the stdout/stderr split.

use std::ffi::OsString;

use clap::error::ErrorKind;

use crate::cli::{Cli, Cmd, Format, PropertyCmd};
use crate::error::CliError;
use crate::render::{self, Document};
use crate::runtime::{self, Log, Runtime};
use crate::schema;

pub struct Finished {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn execute(args: &[OsString], stdout_is_tty: bool) -> Finished {
    let color = runtime::color_enabled(args, stdout_is_tty);
    let cli = match Cli::parse(args, color) {
        Ok(cli) => cli,
        Err(error) => return finish_clap(error, args, stdout_is_tty),
    };
    let runtime = match Runtime::from(&cli.global, stdout_is_tty) {
        Ok(runtime) => runtime,
        Err(error) => {
            let format = if runtime::wants_json_errors(args, stdout_is_tty) {
                Format::Json
            } else {
                Format::Table
            };
            return finish_error(error, format, Log::default());
        }
    };
    let mut log = Log::default();
    match dispatch(cli.command, &runtime, &mut log) {
        Ok((code, document)) => finish_ok(&runtime, log, code, document),
        Err(error) => finish_error(error, runtime.format, log),
    }
}

fn dispatch(command: Cmd, runtime: &Runtime, log: &mut Log) -> Result<(i32, Document), CliError> {
    match command {
        Cmd::Device(cmd) => crate::read_cmd::device(cmd, runtime, log),
        Cmd::Property(cmd) => match cmd {
            PropertyCmd::List { .. } | PropertyCmd::Get { .. } => {
                crate::read_cmd::property(cmd, runtime, log)
            }
            other => crate::write_cmd::property(other, runtime, log),
        },
        Cmd::Control(cmd) => {
            if let Some(done) = crate::read_cmd::control_read(&cmd, runtime, log)? {
                Ok(done)
            } else {
                crate::write_cmd::control(cmd, runtime, log)
            }
        }
        Cmd::Operation(cmd) => crate::read_cmd::operation(cmd, runtime, log),
        Cmd::Event(cmd) => crate::read_cmd::event(cmd, runtime, log),
        Cmd::Storage(cmd) => crate::read_cmd::storage(cmd, runtime, log),
        Cmd::Object(cmd) => crate::read_cmd::object(cmd, runtime, log),
        Cmd::Thumbnail(cmd) => crate::read_cmd::thumbnail(cmd, runtime, log),
        Cmd::Capture(cmd) => crate::write_cmd::capture(cmd, runtime, log),
        Cmd::Zoom(cmd) => crate::write_cmd::zoom(cmd, runtime, log),
        Cmd::Focus(cmd) => crate::write_cmd::focus(cmd, runtime, log),
        Cmd::FocusPoint(cmd) => crate::write_cmd::focus_point(cmd, runtime, log),
        Cmd::Touch(cmd) => crate::write_cmd::touch(cmd, runtime, log),
        Cmd::Movie(cmd) => crate::write_cmd::movie(cmd, runtime, log),
        Cmd::LiveView(cmd) => crate::read_cmd::live_view(cmd, runtime, log),
        Cmd::Schema(args) => schema::run(&args, runtime.format),
        Cmd::Help(args) => schema::help(&args, runtime.format, runtime.color),
    }
}

fn finish_ok(runtime: &Runtime, mut log: Log, code: i32, document: Document) -> Finished {
    if let Document::Many(list) = &document {
        if runtime.stdout_is_tty && !runtime.quiet && list.items.len() < list.total {
            log.warn(
                runtime,
                &format!(
                    "showing {} of {} on page {}. Pass --page {} or --page-all.",
                    list.items.len(),
                    list.total,
                    list.page,
                    list.page + 1
                ),
            );
        }
    }
    Finished {
        code,
        stdout: render::render(&document, runtime.format, runtime.color),
        stderr: log.stderr,
    }
}

fn finish_error(error: CliError, format: Format, log: Log) -> Finished {
    let rendered = if render::errors_are_json(format) {
        render::json_error(&error)
    } else {
        format!("{error}\n")
    };
    Finished {
        code: error.exit,
        stdout: String::new(),
        stderr: format!("{}{rendered}", log.stderr),
    }
}

fn finish_clap(error: clap::Error, args: &[OsString], stdout_is_tty: bool) -> Finished {
    match error.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => Finished {
            code: 0,
            stdout: with_newline(&schema::strip_ansi(&error.render().to_string())),
            stderr: String::new(),
        },
        _ if runtime::wants_json_errors(args, stdout_is_tty) => {
            let cli = clap_error(&error, args);
            Finished {
                code: cli.exit,
                stdout: String::new(),
                stderr: render::json_error(&cli),
            }
        }
        _ => {
            let text = with_newline(&schema::strip_ansi(&error.render().to_string()));
            if error.use_stderr() {
                Finished {
                    code: error.exit_code(),
                    stdout: String::new(),
                    stderr: text,
                }
            } else {
                Finished {
                    code: error.exit_code(),
                    stdout: text,
                    stderr: String::new(),
                }
            }
        }
    }
}

fn clap_error(error: &clap::Error, args: &[OsString]) -> CliError {
    let exit = {
        let code = error.exit_code();
        if code == 0 {
            2
        } else {
            code
        }
    };
    CliError::new(
        "usage",
        schema::plain_clap(error),
        help_suggestion(args),
        false,
        exit,
    )
    .input(
        args.iter()
            .skip(1)
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn help_suggestion(args: &[OsString]) -> String {
    let mut parts = vec!["cameractl".to_owned()];
    for arg in args.iter().skip(1) {
        let text = arg.to_string_lossy();
        if text.starts_with('-') {
            break;
        }
        parts.push(text.into_owned());
    }
    if parts.len() == 1 {
        "Run: cameractl --help".to_owned()
    } else {
        format!("Run: {} --help", parts.join(" "))
    }
}

fn with_newline(text: &str) -> String {
    if text.ends_with('\n') {
        text.to_owned()
    } else {
        format!("{text}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn run(args: &[&str], tty: bool) -> Finished {
        let os: Vec<OsString> = args.iter().map(OsString::from).collect();
        execute(&os, tty)
    }

    fn json_stdout(finished: &Finished) -> Value {
        serde_json::from_str(&finished.stdout)
            .unwrap_or_else(|err| panic!("stdout was not JSON ({err}): {}", finished.stdout))
    }

    #[test]
    fn help_lists_exit_codes_and_schema_is_json_off_a_terminal() {
        let help = run(&["cameractl", "--help"], false);
        assert_eq!(help.code, 0);
        assert!(help.stdout.contains("10"));
        assert!(help.stdout.contains("dry-run"));
        assert!(help.stdout.contains("capture"));
        let schema = run(&["cameractl", "schema", "property", "update"], false);
        assert_eq!(schema.code, 0);
        let body = json_stdout(&schema);
        let flags = body["flags"].as_array().unwrap();
        assert!(flags.iter().any(|flag| flag["name"] == "--dry-run"));
        assert!(flags.iter().any(|flag| {
            flag["name"] == "--format"
                && flag["values"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|value| value == "json")
        }));
        let examples = body["examples"].as_array().unwrap();
        assert!(examples
            .iter()
            .any(|line| line.as_str().unwrap().contains("iso-sensitivity")));
    }

    #[test]
    fn schema_all_has_no_surprise_short_flags() {
        let finished = run(&["cameractl", "schema", "--all"], false);
        assert_eq!(finished.code, 0);
        let body = json_stdout(&finished);
        let mut shorts = Vec::new();
        collect_shorts(&body, &mut shorts);
        assert!(
            shorts.iter().all(|short| short == "-h"),
            "unexpected short flags: {shorts:?}"
        );
        assert!(finished.stdout.contains("focus-point"));
    }

    #[test]
    fn prefixes_unknown_flags_and_bad_keys_fail_before_usb() {
        assert_eq!(run(&["cameractl", "prop"], false).code, 2);
        assert_eq!(run(&["cameractl", "property", "upd"], false).code, 2);
        let format = run(&["cameractl", "schema", "--all", "--format", "yaml"], false);
        assert_eq!(format.code, 2);
        let error: Value = serde_json::from_str(format.stderr.trim()).unwrap();
        assert_eq!(error["error"], "usage");
        assert_eq!(error["retryable"], false);
        let both = run(
            &[
                "cameractl",
                "--json",
                "--format",
                "table",
                "schema",
                "--all",
            ],
            false,
        );
        assert_eq!(both.code, 2);
        let missing = run(
            &[
                "cameractl",
                "property",
                "update",
                "--name",
                "not-a-real-property",
                "--value",
                "1",
            ],
            false,
        );
        assert_eq!(missing.code, 2);
        let stderr: Value = serde_json::from_str(missing.stderr.trim()).unwrap();
        assert_eq!(stderr["input"], "not-a-real-property");
        assert!(
            stderr["message"].as_str().unwrap().contains("Did you mean")
                || stderr["suggestion"]
                    .as_str()
                    .unwrap()
                    .contains("property list")
        );
        let path = run(
            &[
                "cameractl",
                "capture",
                "create",
                "--output",
                "missing/.ssh/id_rsa",
                "--dry-run",
            ],
            false,
        );
        assert_eq!(path.code, 2);
        let key = run(
            &[
                "cameractl",
                "capture",
                "create",
                "--idempotency-key",
                "bad key",
                "--dry-run",
            ],
            false,
        );
        assert_eq!(key.code, 2);
        let page = run(&["cameractl", "property", "list", "--page", "0"], false);
        assert_eq!(page.code, 2);
    }

    #[test]
    fn negative_speed_parses_and_help_stays_compact() {
        let parsed = Cli::parse(
            &[
                OsString::from("cameractl"),
                OsString::from("zoom"),
                OsString::from("update"),
                OsString::from("--speed"),
                OsString::from("-3"),
                OsString::from("--dry-run"),
            ],
            false,
        )
        .unwrap();
        match parsed.command {
            Cmd::Zoom(crate::cli::ZoomCmd::Update { speed, side }) => {
                assert_eq!(speed, -3);
                assert!(side.dry_run);
            }
            other => panic!("unexpected {other:?}"),
        }
        let help = run(&["cameractl", "help", "property", "update"], true);
        assert_eq!(help.code, 0);
        assert!(help.stdout.contains("--dry-run"));
        assert!(
            help.stdout.lines().count() < 60,
            "help is too long:\n{}",
            help.stdout
        );
    }

    #[test]
    fn piped_help_command_is_json() {
        let finished = run(&["cameractl", "help", "device", "get"], false);
        assert_eq!(finished.code, 0);
        let body = json_stdout(&finished);
        assert!(body["help"].as_str().unwrap().contains("device get"));
    }

    fn collect_shorts(value: &Value, shorts: &mut Vec<String>) {
        if let Some(flags) = value.get("flags").and_then(|flags| flags.as_array()) {
            for flag in flags {
                if let Some(short) = flag.get("short").and_then(|short| short.as_str()) {
                    shorts.push(short.to_owned());
                }
            }
        }
        if let Some(children) = value.get("subcommands").and_then(|items| items.as_array()) {
            for child in children {
                collect_shorts(child, shorts);
            }
        }
    }
}
