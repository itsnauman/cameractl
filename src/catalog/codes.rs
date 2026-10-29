//! Wire codes for the PTP namespaces that share a `u16` range.
//!
//! An operation number and a property number are both `u16`, and nothing in the
//! type system stops one from being passed where the other belongs. These
//! newtypes keep the namespaces apart. The inner value is the number on the wire.

/// PTP operation code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OperationCode(pub u16);

/// PTP device-property code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PropertyCode(pub u16);

/// PTP control code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ControlCode(pub u16);

/// PTP event code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EventCode(pub u16);

impl From<OperationCode> for u16 {
    fn from(code: OperationCode) -> Self {
        code.0
    }
}

impl From<PropertyCode> for u16 {
    fn from(code: PropertyCode) -> Self {
        code.0
    }
}

impl From<ControlCode> for u16 {
    fn from(code: ControlCode) -> Self {
        code.0
    }
}

impl From<EventCode> for u16 {
    fn from(code: EventCode) -> Self {
        code.0
    }
}
