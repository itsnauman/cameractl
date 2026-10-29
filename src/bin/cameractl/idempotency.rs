//! Local replay records so a retry does not fire the shutter twice.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::Value;

use crate::error::CliError;

const VERSION: u64 = 1;
const LIMIT: usize = 200;

#[derive(Debug)]
pub enum Begin {
    Replay(Value),
    Run { replay_pending: bool },
}

pub fn begin(key: &Option<String>, command: &str, dry_run: bool) -> Result<Begin, CliError> {
    let Some(key) = key else {
        return Ok(Begin::Run {
            replay_pending: false,
        });
    };
    validate(key)?;
    let _guard = lock();
    match lookup_locked(key, command)? {
        Some(body) if !dry_run => Ok(Begin::Replay(mark_replay(body))),
        Some(_) => Ok(Begin::Run {
            replay_pending: true,
        }),
        None => Ok(Begin::Run {
            replay_pending: false,
        }),
    }
}

pub fn finish(
    key: &Option<String>,
    command: &str,
    dry_run: bool,
    mut body: Value,
    log: &mut crate::runtime::Log,
    runtime: &crate::runtime::Runtime,
) -> Value {
    let Some(key) = key else {
        return body;
    };
    if dry_run {
        body["idempotency_stored"] = Value::Bool(false);
        return body;
    }
    let _guard = lock();
    match store_locked(key, command, &body) {
        Ok(()) => body["idempotency_stored"] = Value::Bool(true),
        Err(error) => {
            log.warn(runtime, &format!("{}. {}", error.message, error.suggestion));
            body["idempotency_stored"] = Value::Bool(false);
        }
    }
    body
}

pub fn validate(key: &str) -> Result<(), CliError> {
    let ok = !key.is_empty()
        && key.len() <= 128
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | ':' | '-'));
    if ok {
        Ok(())
    } else {
        Err(CliError::usage(
            "idempotency key must be 1 to 128 characters of letters, digits, '.', '_', ':', or '-'",
            "Example: --idempotency-key shot-1",
        )
        .input(key))
    }
}

fn mark_replay(mut body: Value) -> Value {
    if let Some(object) = body.as_object_mut() {
        object.insert("idempotent_replay".to_owned(), Value::Bool(true));
        object.insert("replay_pending".to_owned(), Value::Bool(false));
    }
    body
}

struct Entry {
    key: String,
    command: String,
    body: Value,
}

fn lookup_locked(key: &str, command: &str) -> Result<Option<Value>, CliError> {
    let path = store_path()?;
    for entry in load(&path)? {
        if entry.key == key {
            if entry.command != command {
                return Err(CliError::conflict(
                    format!(
                        "idempotency key {key} was already used for {}",
                        entry.command
                    ),
                    "Pass a new --idempotency-key for this command.",
                )
                .input(key));
            }
            return Ok(Some(entry.body));
        }
    }
    Ok(None)
}

fn store_locked(key: &str, command: &str, body: &Value) -> Result<(), CliError> {
    let path = store_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| crate::error::io(&err, "create", parent))?;
    }
    let mut entries = load(&path)?;
    if entries.iter().any(|entry| entry.key == key) {
        return Ok(());
    }
    if entries.len() >= LIMIT {
        entries.remove(0);
    }
    entries.push(Entry {
        key: key.to_owned(),
        command: command.to_owned(),
        body: body.clone(),
    });
    save(&path, &entries)
}

fn load(path: &std::path::Path) -> Result<Vec<Entry>, CliError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path).map_err(|err| crate::error::io(&err, "read", path))?;
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let value: Value =
        serde_json::from_str(&text).map_err(|err| corrupt(path, &err.to_string()))?;
    if value.get("version").and_then(Value::as_u64) != Some(VERSION) {
        return Err(corrupt(path, "unsupported version"));
    }
    let Some(rows) = value.get("entries").and_then(Value::as_array) else {
        return Err(corrupt(path, "missing entries"));
    };
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        let key = row
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| corrupt(path, "entry missing key"))?;
        let command = row
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| corrupt(path, "entry missing command"))?;
        let body = row.get("body").cloned().unwrap_or(Value::Null);
        entries.push(Entry {
            key: key.to_owned(),
            command: command.to_owned(),
            body,
        });
    }
    Ok(entries)
}

fn save(path: &std::path::Path, entries: &[Entry]) -> Result<(), CliError> {
    let rows: Vec<Value> = entries
        .iter()
        .map(|entry| {
            serde_json::json!({
                "key": entry.key,
                "command": entry.command,
                "body": entry.body,
            })
        })
        .collect();
    let text = serde_json::to_string(&serde_json::json!({"version": VERSION, "entries": rows}))
        .map_err(|err| corrupt(path, &err.to_string()))?;
    let mut temporary_name = path
        .file_name()
        .unwrap_or(std::ffi::OsStr::new("idempotency.json"))
        .to_os_string();
    temporary_name.push(".partial");
    let temporary = path.with_file_name(temporary_name);
    fs::write(&temporary, text).map_err(|err| crate::error::io(&err, "write", &temporary))?;
    fs::rename(&temporary, path).map_err(|err| {
        let _ = fs::remove_file(&temporary);
        crate::error::io(&err, "rename", path)
    })
}

fn store_path() -> Result<PathBuf, CliError> {
    Ok(state_dir()?.join("idempotency.json"))
}

fn state_dir() -> Result<PathBuf, CliError> {
    if let Some(dir) = std::env::var_os("CAMERACTL_STATE_DIR") {
        if dir.is_empty() {
            return Err(CliError::usage(
                "CAMERACTL_STATE_DIR is empty",
                "Unset it or point it at a writable directory.",
            ));
        }
        return Ok(PathBuf::from(dir));
    }
    if let Some(dir) = std::env::var_os("XDG_STATE_HOME") {
        if !dir.is_empty() {
            return Ok(PathBuf::from(dir).join("cameractl"));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        return Ok(PathBuf::from(home).join(".local/state/cameractl"));
    }
    Ok(std::env::temp_dir().join("cameractl"))
}

fn corrupt(path: &std::path::Path, detail: &str) -> CliError {
    CliError::general(
        format!("idempotency store {} is unusable: {detail}", path.display()),
        format!(
            "Fix or remove {} and retry. Keys in that file will no longer replay.",
            path.display()
        ),
    )
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|err| err.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{Log, Runtime};
    use sony_ptp3::{ConnectOptions, FunctionMode};
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;

    static ENV_LOCK: StdMutex<()> = StdMutex::new(());

    fn runtime() -> Runtime {
        Runtime {
            format: crate::cli::Format::Json,
            color: false,
            verbose: false,
            quiet: true,
            stdout_is_tty: false,
            options: ConnectOptions {
                function_mode: Some(FunctionMode::RemoteControl),
                reset_device: false,
                release_system_client: false,
                timeout: Duration::from_secs(5),
            },
        }
    }

    #[test]
    fn a_repeated_key_replays_and_a_different_command_conflicts() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("cameractl-idemp-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("CAMERACTL_STATE_DIR", &dir);
        let body = serde_json::json!({"action": "create", "ok": true});
        let mut log = Log::default();
        let stored = finish(
            &Some("shot-1".to_owned()),
            "capture create",
            false,
            body,
            &mut log,
            &runtime(),
        );
        assert_eq!(stored["idempotency_stored"], true);
        match begin(&Some("shot-1".to_owned()), "capture create", false).unwrap() {
            Begin::Replay(value) => assert_eq!(value["idempotent_replay"], true),
            Begin::Run { .. } => panic!("expected replay"),
        }
        let conflict = begin(&Some("shot-1".to_owned()), "zoom update", false).unwrap_err();
        assert_eq!(conflict.exit, 5);
        std::env::remove_var("CAMERACTL_STATE_DIR");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn keys_reject_spaces() {
        let error = validate("shot 1").unwrap_err();
        assert_eq!(error.exit, 2);
    }
}
