//! Failures an agent can branch on without reading free text.

use std::fmt::{Display, Formatter};
use std::io;
use std::path::Path;

use sony_ptp3::Error;

pub const EXIT_OK: i32 = 0;
pub const EXIT_GENERAL: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_NOT_FOUND: i32 = 3;
pub const EXIT_PERMISSION: i32 = 4;
pub const EXIT_CONFLICT: i32 = 5;
pub const EXIT_TRANSIENT: i32 = 6;
pub const EXIT_DRY_RUN: i32 = 10;

/// Which command family produced a camera error, so the recovery hint matches it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    General,
    Property,
    Control,
    Media,
    Event,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    pub code: &'static str,
    pub message: String,
    pub suggestion: String,
    pub retryable: bool,
    pub input: Option<String>,
    pub exit: i32,
}

impl CliError {
    pub fn new(
        code: &'static str,
        message: impl Into<String>,
        suggestion: impl Into<String>,
        retryable: bool,
        exit: i32,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            suggestion: suggestion.into(),
            retryable,
            input: None,
            exit,
        }
    }

    pub fn usage(message: impl Into<String>, suggestion: impl Into<String>) -> Self {
        Self::new("usage", message, suggestion, false, EXIT_USAGE)
    }

    pub fn not_found(message: impl Into<String>, suggestion: impl Into<String>) -> Self {
        Self::new("not_found", message, suggestion, false, EXIT_NOT_FOUND)
    }

    pub fn permission(message: impl Into<String>, suggestion: impl Into<String>) -> Self {
        Self::new(
            "permission_denied",
            message,
            suggestion,
            false,
            EXIT_PERMISSION,
        )
    }

    pub fn conflict(message: impl Into<String>, suggestion: impl Into<String>) -> Self {
        Self::new("conflict", message, suggestion, false, EXIT_CONFLICT)
    }

    pub fn transient(message: impl Into<String>, suggestion: impl Into<String>) -> Self {
        Self::new("transient", message, suggestion, true, EXIT_TRANSIENT)
    }

    pub fn general(message: impl Into<String>, suggestion: impl Into<String>) -> Self {
        Self::new("camera_error", message, suggestion, false, EXIT_GENERAL)
    }

    pub fn input(mut self, input: impl Into<String>) -> Self {
        self.input = Some(input.into());
        self
    }

    pub fn from_ptp(error: Error) -> Self {
        Self::from_ptp_in(error, Context::General)
    }

    pub fn from_ptp_in(error: Error, context: Context) -> Self {
        let message = error.to_string();
        let hint = list_hint(context);
        match error {
            Error::CameraNotFound => CliError::not_found(
                message,
                "Connect the camera with USB, set PC Remote (or PC Control), and retry. On macOS the default session stops Image Capture; pass --no-release-system-client only when another process should keep the device.",
            ),
            Error::Timeout | Error::Stalled | Error::MissingResponse => {
                CliError::transient(message, "Reconnect the camera and retry.")
            }
            Error::Usb(err) => from_usb(err, message),
            Error::Unsupported { .. } => CliError::not_found(message, hint),
            Error::NotSettable { .. } => CliError::permission(
                message,
                "Run: cameractl property get --code <code> --format json",
            ),
            Error::PartialRange | Error::Datatype(_) => CliError::usage(
                message,
                "Check the value against cameractl property get or cameractl control get.",
            ),
            Error::Response { code, .. } => match code {
                0x2019 | 0x201e => CliError::transient(message, "Wait briefly and retry."),
                0x2004 => CliError::transient(message, "Retry with --reset-device."),
                0x200f | 0xa101 => CliError::permission(message, hint),
                0x2005 | 0x200a | 0x2009 | 0x2013 => CliError::not_found(message, hint),
                0x201b..=0x201d => CliError::usage(
                    message,
                    "Read the accepted values, then retry. Run: cameractl property get --name <slug> --format json",
                ),
                0x200c => CliError::general(message, "Free space on the memory card, then retry."),
                _ => CliError::general(message, hint),
            },
            _ => CliError::general(message, hint),
        }
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "error": self.code,
            "message": self.message,
            "suggestion": if self.suggestion.is_empty() { serde_json::Value::Null } else { serde_json::json!(self.suggestion) },
            "retryable": self.retryable,
            "input": self.input,
            "exit": self.exit,
        })
    }
}

impl Display for CliError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "error[{}]: {}", self.code, self.message)?;
        if let Some(input) = &self.input {
            write!(formatter, "\ninput: {input}")?;
        }
        if !self.suggestion.is_empty() {
            write!(formatter, "\nsuggestion: {}", self.suggestion)?;
        }
        Ok(())
    }
}

pub fn io(err: &io::Error, action: &str, path: &Path) -> CliError {
    let message = format!("{action} {}: {err}", path.display());
    match err.kind() {
        io::ErrorKind::PermissionDenied => CliError::permission(
            message,
            "Choose a directory you can write, such as the current directory.",
        ),
        io::ErrorKind::NotFound => {
            CliError::not_found(message, "Create the directory, or pick an existing one.")
        }
        io::ErrorKind::Interrupted | io::ErrorKind::TimedOut => {
            CliError::transient(message, "Retry the command.")
        }
        _ => CliError::general(message, "Check the path and retry."),
    }
}

fn from_usb(err: rusb::Error, message: String) -> CliError {
    match err {
        rusb::Error::Access => CliError::permission(
            message,
            "Allow this process to use USB. On macOS, retry without --no-release-system-client so Image Capture releases the camera.",
        ),
        rusb::Error::Timeout
        | rusb::Error::Busy
        | rusb::Error::Pipe
        | rusb::Error::Interrupted
        | rusb::Error::Io
        | rusb::Error::Overflow
        | rusb::Error::NoDevice => CliError::transient(message, "Reconnect the camera and retry."),
        rusb::Error::NotFound | rusb::Error::NotSupported => CliError::not_found(
            message,
            "Set the camera to PC Remote mode and reconnect USB.",
        ),
        _ => CliError::general(message, "Reconnect the camera and run: cameractl device get"),
    }
}

fn list_hint(context: Context) -> String {
    match context {
        Context::Property => "Run: cameractl property list --format json".to_owned(),
        Context::Control => "Run: cameractl control list --format json".to_owned(),
        Context::Media => "Card access often needs an SDIO session. Run: cameractl object list --function-mode content-transfer".to_owned(),
        Context::Event => "Run: cameractl event list --format json".to_owned(),
        Context::General => "Run: cameractl device get".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_absence_is_not_found_and_not_retryable() {
        let error = CliError::from_ptp(Error::CameraNotFound);
        assert_eq!(error.exit, EXIT_NOT_FOUND);
        assert!(!error.retryable);
        assert_eq!(error.code, "not_found");
        assert!(error.suggestion.contains("PC Remote"));
    }

    #[test]
    fn timeout_is_retryable() {
        let error = CliError::from_ptp(Error::Timeout);
        assert_eq!(error.exit, EXIT_TRANSIENT);
        assert!(error.retryable);
    }

    #[test]
    fn device_busy_is_retryable() {
        let error = CliError::from_ptp(Error::Response {
            operation: 0x9205,
            code: 0x2019,
            name: "Device Busy",
        });
        assert_eq!(error.exit, EXIT_TRANSIENT);
        assert!(error.retryable);
    }

    #[test]
    fn unsupported_property_points_at_the_property_list() {
        let error = CliError::from_ptp_in(
            Error::Unsupported {
                model: "ZV-E10".into(),
                kind: sony_ptp3::Capability::Property,
                code: 0xd21e,
                name: "ISO Sensitivity",
            },
            Context::Property,
        );
        assert_eq!(error.exit, EXIT_NOT_FOUND);
        assert!(error.suggestion.contains("property list"));
    }
}
