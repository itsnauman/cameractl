//! Failures from the USB session and from PTP dataset decoding.

use std::fmt::{Display, Formatter};

use crate::catalog;
use crate::catalog::{ControlCode, OperationCode, PropertyCode};

/// Which advertised list a request was checked against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// [`crate::codec::DeviceInfo::operations`].
    Operation,
    /// Device properties from the handshake and from `SDIO_GetAllExtDevicePropInfo`.
    Property,
    /// [`crate::codec::ExtDeviceInfo::controls`].
    Control,
}

impl Display for Capability {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Operation => "operation",
            Self::Property => "property",
            Self::Control => "control",
        })
    }
}

/// Failure while talking to the camera or decoding a dataset.
#[must_use]
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("USB error: {0}")]
    Usb(#[from] rusb::Error),
    #[error("no Sony camera in PTP mode was found on USB")]
    CameraNotFound,
    #[error("the PTP interface has no bulk endpoints")]
    MissingEndpoints,
    #[error("{model} does not support {kind} {name} ({code:#06x})")]
    Unsupported {
        model: String,
        kind: Capability,
        code: u16,
        name: &'static str,
    },
    #[error("PTP session is closed")]
    SessionClosed,
    #[error("timed out waiting for the camera")]
    Timeout,
    #[error("the camera stalled the USB pipe")]
    Stalled,
    #[error("truncated PTP payload")]
    Truncated,
    #[error("live view dataset is shorter than 16 bytes")]
    LiveViewHeader,
    #[error("{what} has {len} trailing bytes")]
    TrailingBytes { what: &'static str, len: usize },
    #[error("{what} length {len} is not usable")]
    InvalidLength { what: &'static str, len: u64 },
    #[error("UINT32 array is {len} bytes: {prefix:02x?}")]
    Uint32Array { len: usize, prefix: Vec<u8> },
    #[error("UINT32 array count {count} does not match {len} bytes")]
    Uint32Count { count: usize, len: usize },
    #[error("property {code:#06x} has form flag {flag:#x}")]
    PropertyForm { code: u16, flag: u8 },
    #[error("{what} offset overflows")]
    OffsetOverflow { what: &'static str },
    #[error("{what} exceeds the dataset")]
    OutOfBounds { what: &'static str },
    #[error("PTP string is not UTF-16")]
    InvalidString,
    #[error("PTP string is longer than 255 code units")]
    StringTooLong,
    #[error("expected a PTP string")]
    ExpectedString,
    #[error("unexpected PTP container type {kind}")]
    ContainerType { kind: u16 },
    #[error("PTP container length {len} is not usable")]
    ContainerLength { len: usize },
    #[error("PTP payload is too large")]
    PayloadTooLarge,
    #[error("the camera did not return a response container")]
    MissingResponse,
    #[error("{what} was not returned")]
    MissingParameter { what: &'static str },
    #[error("SDIO_GetPartialLargeObject does not accept 0xffffffff")]
    PartialRange,
    #[error("datatype {0:#06x} is not a PTP value")]
    Datatype(u16),
    #[error("property {code:#06x} cannot be changed")]
    NotSettable { code: u16 },
    #[error("operation {operation:#06x} was rejected with {code:#06x} ({name})")]
    Response {
        operation: u16,
        code: u16,
        name: &'static str,
    },
}

/// Name of a PTP or Sony response code.
///
/// Standard PTP codes are named here. Other codes use [`catalog::response`]
/// when the reference lists them.
#[must_use]
pub fn response_name(code: u16) -> &'static str {
    match code {
        0x2001 => "OK",
        0x2002 => "General Error",
        0x2003 => "Session Not Open",
        0x2004 => "Invalid Transaction ID",
        0x2005 => "Operation Not Supported",
        0x2006 => "Parameter Not Supported",
        0x2007 => "Incomplete Transfer",
        0x2009 => "Invalid Object Handle",
        0x200a => "Device Prop Not Supported",
        0x200b => "Invalid Object Format",
        0x200c => "Store Full",
        0x200f => "Access Denied",
        0x2010 => "No Thumbnail Present",
        0x2013 => "Store Not Available",
        0x2019 => "Device Busy",
        0x201b => "Invalid Device Prop Format",
        0x201c => "Invalid Device Prop Value",
        0x201d => "Invalid Parameter",
        0x201e => "Session Already Open",
        0xa101 => "Authentication Failed",
        0xa102 => "Password Length Over Max",
        0xa103 => "Password Includes Invalid Character",
        0xa104 => "Feature Version Invalid",
        0xa105 => "Temporary Storage Full",
        0xa106 => "Camera Status Error",
        other => catalog::response(other)
            .map(|spec| spec.name)
            .unwrap_or("response"),
    }
}

impl Error {
    pub(crate) fn rejected(operation: u16, code: u16) -> Self {
        Self::Response {
            operation,
            code,
            name: response_name(code),
        }
    }

    pub(crate) fn unsupported(model: &str, kind: Capability, code: u16) -> Self {
        let name = match kind {
            Capability::Operation => catalog::operation(OperationCode(code)).map(|spec| spec.name),
            Capability::Property => catalog::property(PropertyCode(code)).map(|spec| spec.name),
            Capability::Control => catalog::control(ControlCode(code)).map(|spec| spec.name),
        }
        .unwrap_or("unnamed");
        Self::Unsupported {
            model: model.to_owned(),
            kind,
            code,
            name,
        }
    }
}
