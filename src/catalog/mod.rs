//! PTP 3 operation, property, control, and event codes.
//!
//! Generated from the PTP 3 reference by `tools/generate_catalog.py`.
//! Edit the generator, not the generated leaves.
//!
//! [`operation`], [`property`], [`event`], [`response`], [`object_format`],
//! and [`control`] binary-search tables sorted by code. [`value_label`]
//! binary-searches rows sorted by property code, then by value.
//! [`property_labels`] walks the labels of one property.

pub mod code;
mod codes;
mod tables;

pub use codes::{ControlCode, EventCode, OperationCode, PropertyCode};
pub use tables::{
    control, event, object_format, operation, property, property_labels, response, value_label,
    ControlSpec, Spec, CONTROLS, EVENTS, OBJECT_FORMATS, OPERATIONS, PROPERTIES, RESPONSES,
};
