//! On-demand command schema. Agents query one command instead of loading every flag.

use std::ffi::OsString;

use clap::{Arg, Command, CommandFactory};

use crate::cli::{Cli, Format, HelpArgs, SchemaArgs};
use crate::error::{CliError, EXIT_OK};
use crate::names;
use crate::present::Column;
use crate::render::{Document, ListOut};

const COMMAND_COLUMNS: &[Column] = &[
    Column {
        key: "command",
        header: "command",
    },
    Column {
        key: "about",
        header: "about",
    },
];

const FLAG_COLUMNS: &[Column] = &[
    Column {
        key: "name",
        header: "flag",
    },
    Column {
        key: "value_type",
        header: "type",
    },
    Column {
        key: "required",
        header: "required",
    },
    Column {
        key: "values",
        header: "values",
    },
    Column {
        key: "help",
        header: "help",
    },
];

pub fn run(args: &SchemaArgs, format: Format) -> Result<(i32, Document), CliError> {
    if args.all && (args.group.is_some() || args.verb.is_some()) {
        return Err(CliError::usage(
            "--all does not take a command path",
            "Run: cameractl schema --all",
        ));
    }
    if !args.all && args.group.is_none() {
        return Err(CliError::usage(
            "pass a command or --all",
            "Run: cameractl schema --all\nOr: cameractl schema property update",
        ));
    }
    let path = command_path(&args.group, &args.verb)?;
    let root = Cli::command();
    let globals = global_flags(&root);
    let value = if args.all {
        node(&root, true, &globals)
    } else {
        node(find(&root, &path)?, false, &globals)
    };
    Ok((EXIT_OK, view(value, format, args.all)))
}

pub fn help(args: &HelpArgs, format: Format, color: bool) -> Result<(i32, Document), CliError> {
    let path = command_path(&args.group, &args.verb)?;
    let root = Cli::command();
    find(&root, &path)?;
    let mut argv = vec![OsString::from("cameractl")];
    argv.extend(path.iter().map(OsString::from));
    argv.push(OsString::from("--help"));
    let text = match Cli::parse(&argv, color) {
        Err(error) if error.kind() == clap::error::ErrorKind::DisplayHelp => render_clap(&error),
        Err(error) => {
            return Err(CliError::usage(
                plain_clap(&error),
                "Run: cameractl schema --all",
            ))
        }
        Ok(_) => {
            return Err(CliError::usage(
                "help produced no text",
                "Run: cameractl --help",
            ))
        }
    };
    let document = match format {
        Format::Table => Document::Text(text),
        Format::Json | Format::Ndjson | Format::Csv => {
            Document::One(serde_json::json!({ "help": text.trim_end() }))
        }
    };
    Ok((EXIT_OK, document))
}

fn view(value: serde_json::Value, format: Format, all: bool) -> Document {
    match format {
        Format::Json => Document::One(value),
        Format::Ndjson => {
            let mut rows = Vec::new();
            flatten_full(&value, "", &mut rows);
            Document::Many(ListOut::full(rows, &[]))
        }
        Format::Table | Format::Csv => {
            let grouped = value
                .get("subcommands")
                .and_then(|items| items.as_array())
                .is_some_and(|items| !items.is_empty());
            if grouped {
                let mut rows = Vec::new();
                if all {
                    flatten_brief(&value, "", &mut rows);
                } else {
                    one_level(&value, &mut rows);
                }
                Document::Many(ListOut::full(rows, COMMAND_COLUMNS))
            } else {
                let flags = value
                    .get("flags")
                    .and_then(|items| items.as_array())
                    .cloned()
                    .unwrap_or_default();
                let rows = flags.iter().map(flag_row).collect();
                Document::Many(ListOut::full(rows, FLAG_COLUMNS))
            }
        }
    }
}

fn node(cmd: &Command, recurse: bool, globals: &[serde_json::Value]) -> serde_json::Value {
    let subcommands: Vec<serde_json::Value> = if recurse {
        cmd.get_subcommands()
            .map(|child| node(child, true, globals))
            .collect()
    } else {
        cmd.get_subcommands()
            .map(|child| {
                serde_json::json!({
                    "name": child.get_name(),
                    "about": about(child),
                    "examples": examples(child),
                })
            })
            .collect()
    };
    serde_json::json!({
        "name": cmd.get_name(),
        "about": about(cmd),
        "examples": examples(cmd),
        "flags": with_globals(cmd, globals),
        "subcommands": subcommands,
    })
}

fn global_flags(root: &Command) -> Vec<serde_json::Value> {
    root.get_arguments()
        .filter(|arg| arg.is_global_set())
        .map(flag)
        .collect()
}

fn with_globals(cmd: &Command, globals: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut flags = flags(cmd);
    if cmd.get_name() == "cameractl" {
        return flags;
    }
    for global in globals {
        let name = global
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if !flags
            .iter()
            .any(|flag| flag.get("name").and_then(|value| value.as_str()) == Some(name))
        {
            flags.push(global.clone());
        }
    }
    flags
}

fn flags(cmd: &Command) -> Vec<serde_json::Value> {
    cmd.get_arguments().map(flag).collect()
}

fn flag(arg: &Arg) -> serde_json::Value {
    let takes_value = arg.get_action().takes_values();
    let long = arg.get_long();
    let name = if let Some(long) = long {
        format!("--{long}")
    } else {
        arg.get_id().as_str().to_owned()
    };
    let values: Vec<String> = arg
        .get_possible_values()
        .iter()
        .map(|value| value.get_name().to_owned())
        .collect();
    let defaults: Vec<String> = arg
        .get_default_values()
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    let default = match defaults.as_slice() {
        [] => serde_json::Value::Null,
        [one] => serde_json::json!(one),
        many => serde_json::json!(many),
    };
    serde_json::json!({
        "name": name,
        "short": arg.get_short().map(|ch| format!("-{ch}")),
        "kind": if !takes_value {
            if long.is_none() { "positional" } else { "flag" }
        } else if long.is_none() {
            "positional"
        } else {
            "option"
        },
        "value_type": if takes_value { "string" } else { "boolean" },
        "required": arg.is_required_set(),
        "default": default,
        "values": values,
        "help": arg.get_help().map(|text| text.to_string()).unwrap_or_default(),
        "hidden": arg.is_hide_set(),
    })
}

fn flag_row(flag: &serde_json::Value) -> serde_json::Value {
    let values = flag
        .get("values")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|value| value.as_str())
                .collect::<Vec<_>>()
                .join("|")
        })
        .unwrap_or_default();
    serde_json::json!({
        "name": flag.get("name").cloned().unwrap_or(serde_json::Value::Null),
        "value_type": flag.get("value_type").cloned().unwrap_or(serde_json::Value::Null),
        "required": flag.get("required").cloned().unwrap_or(serde_json::Value::Bool(false)),
        "values": values,
        "help": flag.get("help").cloned().unwrap_or(serde_json::Value::Null),
    })
}

fn flatten_full(value: &serde_json::Value, prefix: &str, rows: &mut Vec<serde_json::Value>) {
    let path = join_path(
        prefix,
        value
            .get("name")
            .and_then(|name| name.as_str())
            .unwrap_or(""),
    );
    rows.push(serde_json::json!({
        "command": path,
        "about": value.get("about").cloned().unwrap_or(serde_json::Value::Null),
        "examples": value.get("examples").cloned().unwrap_or(serde_json::json!([])),
        "flags": value.get("flags").cloned().unwrap_or(serde_json::json!([])),
    }));
    if let Some(children) = value.get("subcommands").and_then(|items| items.as_array()) {
        for child in children {
            flatten_full(child, &path, rows);
        }
    }
}

fn flatten_brief(value: &serde_json::Value, prefix: &str, rows: &mut Vec<serde_json::Value>) {
    let path = join_path(
        prefix,
        value
            .get("name")
            .and_then(|name| name.as_str())
            .unwrap_or(""),
    );
    rows.push(serde_json::json!({
        "command": path,
        "about": value.get("about").cloned().unwrap_or(serde_json::Value::Null),
    }));
    if let Some(children) = value.get("subcommands").and_then(|items| items.as_array()) {
        for child in children {
            flatten_brief(child, &path, rows);
        }
    }
}

fn one_level(value: &serde_json::Value, rows: &mut Vec<serde_json::Value>) {
    let prefix = value
        .get("name")
        .and_then(|name| name.as_str())
        .unwrap_or("");
    let Some(children) = value.get("subcommands").and_then(|items| items.as_array()) else {
        return;
    };
    for child in children {
        let name = child
            .get("name")
            .and_then(|name| name.as_str())
            .unwrap_or("");
        rows.push(serde_json::json!({
            "command": format!("{prefix} {name}"),
            "about": child.get("about").cloned().unwrap_or(serde_json::Value::Null),
        }));
    }
}

fn join_path(prefix: &str, name: &str) -> String {
    if prefix.is_empty() || (prefix == "cameractl" && name == "cameractl") {
        name.to_owned()
    } else {
        format!("{prefix} {name}")
    }
}

fn find<'a>(cmd: &'a Command, path: &[String]) -> Result<&'a Command, CliError> {
    let mut current = cmd;
    for name in path {
        let Some(child) = current.find_subcommand(name) else {
            let choices: Vec<&str> = current
                .get_subcommands()
                .map(|child| child.get_name())
                .collect();
            let suggestions = names::closest(name, choices.into_iter());
            let message = if suggestions.is_empty() {
                format!("unknown command {name}")
            } else {
                format!(
                    "unknown command {name}. Did you mean {}?",
                    suggestions.join(", ")
                )
            };
            return Err(CliError::usage(message, "Run: cameractl schema --all").input(name.clone()));
        };
        current = child;
    }
    Ok(current)
}

pub fn command_path(
    group: &Option<String>,
    verb: &Option<String>,
) -> Result<Vec<String>, CliError> {
    match (group, verb) {
        (None, None) => Ok(Vec::new()),
        (Some(group), None) => Ok(vec![group.clone()]),
        (Some(group), Some(verb)) => Ok(vec![group.clone(), verb.clone()]),
        (None, Some(_)) => Err(CliError::usage(
            "pass the command name before the verb",
            "Example: cameractl schema property update",
        )),
    }
}

fn about(cmd: &Command) -> String {
    cmd.get_about()
        .map(|text| text.to_string())
        .unwrap_or_default()
}

fn examples(cmd: &Command) -> Vec<String> {
    cmd.get_before_help()
        .map(|text| text.to_string())
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("cameractl "))
        .map(str::to_owned)
        .collect()
}

pub fn plain_clap(error: &clap::Error) -> String {
    let text = strip_ansi(&error.render().to_string());
    let trimmed = text.trim();
    if trimmed.lines().count() > 12 {
        let first = trimmed.lines().next().unwrap_or(trimmed);
        format!("{first} Run the suggested help command for the full usage text.")
    } else {
        trimmed.to_owned()
    }
}

pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

fn render_clap(error: &clap::Error) -> String {
    let text = strip_ansi(&error.render().to_string());
    if text.ends_with('\n') {
        text
    } else {
        format!("{text}\n")
    }
}
