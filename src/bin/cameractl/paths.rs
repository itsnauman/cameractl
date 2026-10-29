//! Output paths. Bytes never go to stdout, and sensitive directories are refused.

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::error::CliError;

pub fn validate(path: &Path) -> Result<PathBuf, CliError> {
    if path.as_os_str().is_empty() {
        return Err(CliError::usage(
            "--output is empty",
            "Pass a file path, for example shot.jpg.",
        ));
    }
    if path.as_os_str().len() > 4096 {
        return Err(CliError::usage(
            "--output is longer than 4096 bytes",
            "Pass a shorter path.",
        ));
    }
    let Some(text) = path.to_str() else {
        return Err(CliError::usage(
            "--output must be valid UTF-8",
            "Pass a UTF-8 path.",
        ));
    };
    if has_sensitive(path) {
        return Err(sensitive(text));
    }
    let Some(file_name) = path.file_name() else {
        return Err(CliError::usage(
            "--output needs a file name",
            "Pass a file path, for example shot.jpg.",
        )
        .input(text));
    };
    if file_name == "." || file_name == ".." {
        return Err(CliError::usage(
            "--output needs a file name",
            "Pass a file path, for example shot.jpg.",
        )
        .input(text));
    }
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    if !parent.exists() {
        return Err(CliError::not_found(
            format!("directory {} does not exist", parent.display()),
            "Create that directory, or pass --output in an existing directory.",
        )
        .input(text));
    }
    if !parent.is_dir() {
        return Err(CliError::usage(
            format!("{} is not a directory", parent.display()),
            "Pass --output inside an existing directory.",
        )
        .input(text));
    }
    let parent = parent
        .canonicalize()
        .map_err(|err| crate::error::io(&err, "open", parent))?;
    if has_sensitive(&parent) {
        return Err(sensitive(text));
    }
    let full = parent.join(file_name);
    if full.is_dir() {
        return Err(CliError::usage(
            format!("{} is a directory", full.display()),
            "Pass a file path, for example shot.jpg.",
        )
        .input(text));
    }
    if full.exists() {
        if let Ok(canonical) = full.canonicalize() {
            if has_sensitive(&canonical) {
                return Err(sensitive(text));
            }
        }
    }
    Ok(full)
}

pub fn write_bytes(path: &Path, bytes: &[u8]) -> Result<PathBuf, CliError> {
    let path = validate(path)?;
    let mut temporary_name = path
        .file_name()
        .unwrap_or(std::ffi::OsStr::new("output"))
        .to_os_string();
    temporary_name.push(".partial");
    let temporary = path.with_file_name(temporary_name);
    if let Err(err) = fs::write(&temporary, bytes) {
        let _ = fs::remove_file(&temporary);
        return Err(crate::error::io(&err, "write", &temporary));
    }
    if let Err(err) = fs::rename(&temporary, &path) {
        let _ = fs::remove_file(&temporary);
        return Err(crate::error::io(&err, "rename", &path));
    }
    Ok(path)
}

fn has_sensitive(path: &Path) -> bool {
    path.components().any(|component| match component {
        Component::Normal(name) => matches!(name.to_str(), Some(".ssh" | ".gnupg" | ".aws")),
        _ => false,
    })
}

fn sensitive(path: &str) -> CliError {
    CliError::usage(
        format!("{path} is inside a sensitive directory"),
        "Pass --output under the current directory, for example shot.jpg.",
    )
    .input(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_directories_are_rejected_before_missing_parents() {
        let error = validate(Path::new("missing/.ssh/id_rsa")).unwrap_err();
        assert_eq!(error.exit, 2);
        assert!(error.message.contains("sensitive"));
    }

    #[test]
    fn a_file_in_an_existing_directory_is_accepted() {
        let dir = std::env::temp_dir().join(format!("cameractl-path-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = validate(&dir.join("shot.jpg")).unwrap();
        assert!(path.ends_with("shot.jpg"));
        assert!(path.is_absolute());
        let _ = fs::remove_dir_all(&dir);
    }
}
