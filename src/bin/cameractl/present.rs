//! JSON shapes. Adding a field is compatible; renaming one is not.

use sony_ptp3::catalog::{self, PropertyCode};
use sony_ptp3::{
    Camera, DeviceProperty, EnableState, Event, GetSet, ObjectInfo, PropForm, PropValue,
    StorageInfo,
};

use crate::names::{self, CatalogEntry};
use crate::parse::{code16, code32};
use crate::values::datatype_name;

#[derive(Clone, Copy, Debug)]
pub struct Column {
    pub key: &'static str,
    pub header: &'static str,
}

pub const PROPERTY_COLUMNS: &[Column] = &[
    Column {
        key: "code",
        header: "code",
    },
    Column {
        key: "slug",
        header: "slug",
    },
    Column {
        key: "name",
        header: "name",
    },
    Column {
        key: "datatype",
        header: "datatype",
    },
    Column {
        key: "writable",
        header: "writable",
    },
    Column {
        key: "current",
        header: "current",
    },
    Column {
        key: "current_label",
        header: "label",
    },
];

pub const CODE_COLUMNS: &[Column] = &[
    Column {
        key: "code",
        header: "code",
    },
    Column {
        key: "slug",
        header: "slug",
    },
    Column {
        key: "name",
        header: "name",
    },
];

pub const CONTROL_COLUMNS: &[Column] = &[
    Column {
        key: "code",
        header: "code",
    },
    Column {
        key: "slug",
        header: "slug",
    },
    Column {
        key: "name",
        header: "name",
    },
    Column {
        key: "datatype",
        header: "datatype",
    },
    Column {
        key: "destructive",
        header: "destructive",
    },
];

pub const STORAGE_COLUMNS: &[Column] = &[
    Column {
        key: "storage_id",
        header: "storage_id",
    },
    Column {
        key: "description",
        header: "description",
    },
    Column {
        key: "volume_label",
        header: "volume_label",
    },
    Column {
        key: "free_bytes",
        header: "free_bytes",
    },
    Column {
        key: "free_images",
        header: "free_images",
    },
];

pub const OBJECT_COLUMNS: &[Column] = &[
    Column {
        key: "handle",
        header: "handle",
    },
    Column {
        key: "storage_id",
        header: "storage_id",
    },
];

pub const OBJECT_DETAIL_COLUMNS: &[Column] = &[
    Column {
        key: "handle",
        header: "handle",
    },
    Column {
        key: "storage_id",
        header: "storage_id",
    },
    Column {
        key: "filename",
        header: "filename",
    },
    Column {
        key: "format_name",
        header: "format",
    },
    Column {
        key: "compressed_size",
        header: "bytes",
    },
    Column {
        key: "image_width",
        header: "width",
    },
    Column {
        key: "image_height",
        header: "height",
    },
    Column {
        key: "capture_date",
        header: "captured",
    },
];

pub fn device_json(camera: &Camera, session_mode: &str) -> serde_json::Value {
    let info = camera.device_info();
    let extended = camera.ext_device_info();
    serde_json::json!({
        "manufacturer": info.manufacturer,
        "model": info.model,
        "device_version": info.device_version,
        "serial_number": info.serial_number,
        "session_mode": session_mode,
        "standard_version": code16(info.standard_version),
        "vendor_extension_id": code32(info.vendor_extension_id),
        "vendor_extension_version": code16(info.vendor_extension_version),
        "extension_version": code16(extended.extension_version),
        "vendor_code_version": code32(extended.vendor_code_version),
        "functional_mode": code16(info.functional_mode),
        "operation_count": info.operations.len(),
        "event_count": info.events.len(),
        "property_count": camera.properties().len(),
        "control_count": extended.controls.len(),
        "capture_format_count": info.capture_formats.len(),
        "image_format_count": info.image_formats.len(),
    })
}

pub fn property_json(property: &DeviceProperty) -> serde_json::Value {
    let entry = names::find(names::properties(), property.code);
    let mut value = serde_json::json!({
        "code": code16(property.code),
        "name": entry.map(|entry| entry.name),
        "slug": entry.map(|entry| entry.slug.as_str()),
        "datatype": datatype_name(property.datatype),
        "get_set": get_set_name(property.get_set),
        "enabled": enabled_name(property.enabled),
        "writable": property.is_writable(),
        "current": prop_json(&property.current),
        "current_label": label_of(property.code, &property.current),
        "factory": prop_json(&property.factory),
        "factory_label": label_of(property.code, &property.factory),
    });
    match &property.form {
        PropForm::None => {
            value["form"] = serde_json::json!("none");
            null_range(&mut value);
            value["values"] = serde_json::json!([]);
            value["read_values"] = serde_json::json!([]);
        }
        PropForm::Range { min, max, step } => {
            value["form"] = serde_json::json!("range");
            value["minimum"] = prop_json(min);
            value["minimum_label"] = serde_json::json!(label_of(property.code, min));
            value["maximum"] = prop_json(max);
            value["maximum_label"] = serde_json::json!(label_of(property.code, max));
            value["step"] = prop_json(step);
            value["step_label"] = serde_json::json!(label_of(property.code, step));
            value["values"] = serde_json::json!([]);
            value["read_values"] = serde_json::json!([]);
        }
        PropForm::Enumeration { set, get_set } => {
            value["form"] = serde_json::json!("enumeration");
            null_range(&mut value);
            value["values"] = serde_json::json!(choices(property.code, set));
            value["read_values"] = serde_json::json!(choices(property.code, get_set));
        }
    }
    value
}

pub fn control_json(code: u16) -> serde_json::Value {
    let entry = names::find(names::controls(), code);
    let datatype = entry.map(|entry| entry.datatype).unwrap_or(0);
    let mut value = identity(names::controls(), code);
    value["datatype"] = serde_json::json!(datatype_name(datatype));
    value["destructive"] = serde_json::json!(names::control_is_destructive(code));
    value
}

pub fn identity(entries: &[CatalogEntry], code: u16) -> serde_json::Value {
    let entry = names::find(entries, code);
    serde_json::json!({
        "code": code16(code),
        "name": entry.map(|entry| entry.name),
        "slug": entry.map(|entry| entry.slug.as_str()),
    })
}

pub fn storage_json(id: u32, info: &StorageInfo) -> serde_json::Value {
    serde_json::json!({
        "storage_id": code32(id),
        "storage_type": code16(info.storage_type),
        "filesystem_type": code16(info.filesystem_type),
        "access": code16(info.access),
        "max_capacity": info.max_capacity,
        "free_bytes": info.free_bytes,
        "free_images": info.free_images,
        "description": info.description,
        "volume_label": info.volume_label,
    })
}

pub fn object_json(handle: u32, info: &ObjectInfo) -> serde_json::Value {
    serde_json::json!({
        "handle": code32(handle),
        "storage_id": code32(info.storage_id),
        "filename": info.filename,
        "format": code16(info.format),
        "format_name": catalog::object_format(info.format).map(|spec| spec.name),
        "compressed_size": info.compressed_size,
        "image_width": info.image_width,
        "image_height": info.image_height,
        "parent": code32(info.parent),
        "capture_date": info.capture_date,
        "modification_date": info.modification_date,
        "protection": info.protection,
        "sequence_number": info.sequence_number,
    })
}

pub fn event_json(event: &Event) -> serde_json::Value {
    let mut value = identity(names::events(), event.code);
    value["params"] = serde_json::json!(event.params);
    value
}

pub fn prop_json(value: &PropValue) -> serde_json::Value {
    match value {
        PropValue::I8(value) => serde_json::json!(value),
        PropValue::U8(value) => serde_json::json!(value),
        PropValue::I16(value) => serde_json::json!(value),
        PropValue::U16(value) => serde_json::json!(value),
        PropValue::I32(value) => serde_json::json!(value),
        PropValue::U32(value) => serde_json::json!(value),
        PropValue::I64(value) => serde_json::json!(value),
        PropValue::U64(value) => serde_json::json!(value),
        PropValue::I128(value) => wide_i128(*value),
        PropValue::U128(value) => wide_u128(*value),
        PropValue::Str(value) => serde_json::json!(value),
        PropValue::I8Array(value) => serde_json::json!(value),
        PropValue::U8Array(value) => serde_json::json!(value),
        PropValue::I16Array(value) => serde_json::json!(value),
        PropValue::U16Array(value) => serde_json::json!(value),
        PropValue::I32Array(value) => serde_json::json!(value),
        PropValue::U32Array(value) => serde_json::json!(value),
        PropValue::I64Array(value) => serde_json::json!(value),
        PropValue::U64Array(value) => serde_json::json!(value),
        PropValue::I128Array(value) => {
            serde_json::Value::Array(value.iter().copied().map(wide_i128).collect())
        }
        PropValue::U128Array(value) => {
            serde_json::Value::Array(value.iter().copied().map(wide_u128).collect())
        }
    }
}

pub fn label_of(code: u16, value: &PropValue) -> Option<&'static str> {
    value
        .label_keys()
        .find_map(|key| catalog::value_label(PropertyCode(code), key))
}

pub fn display_value(code: u16, value: &PropValue) -> String {
    match label_of(code, value) {
        Some(label) => format!("{label} ({value})"),
        None => value.to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn mutation(
    action: &str,
    resource: &str,
    dry_run: bool,
    writes: bool,
    changed: bool,
    reversible: bool,
    key: Option<&str>,
    replay_pending: bool,
    target: serde_json::Value,
    changes: Vec<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "action": action,
        "resource": resource,
        "dry_run": dry_run,
        "writes": writes,
        "changed": changed,
        "reversible": reversible,
        "idempotency_key": key,
        "idempotent_replay": false,
        "idempotency_stored": serde_json::Value::Null,
        "replay_pending": replay_pending,
        "target": target,
        "changes": changes,
    })
}

pub fn change(
    field: &str,
    from: serde_json::Value,
    from_label: Option<&str>,
    to: serde_json::Value,
    to_label: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "field": field,
        "from": from,
        "from_label": from_label,
        "to": to,
        "to_label": to_label,
    })
}

pub fn value_change(field: &str, code: u16, from: &PropValue, to: &PropValue) -> serde_json::Value {
    change(
        field,
        prop_json(from),
        label_of(code, from),
        prop_json(to),
        label_of(code, to),
    )
}

fn choices(code: u16, values: &[PropValue]) -> Vec<serde_json::Value> {
    values
        .iter()
        .map(|value| {
            serde_json::json!({
                "value": prop_json(value),
                "label": label_of(code, value),
            })
        })
        .collect()
}

fn null_range(value: &mut serde_json::Value) {
    for key in [
        "minimum",
        "minimum_label",
        "maximum",
        "maximum_label",
        "step",
        "step_label",
    ] {
        value[key] = serde_json::Value::Null;
    }
}

fn get_set_name(value: GetSet) -> &'static str {
    match value {
        GetSet::ReadOnly => "read-only",
        GetSet::ReadWrite => "read-write",
        GetSet::Other(_) => "other",
    }
}

fn enabled_name(value: EnableState) -> &'static str {
    match value {
        EnableState::Disabled => "disabled",
        EnableState::Enabled => "enabled",
        EnableState::DisplayOnly => "display-only",
        EnableState::Other(_) => "other",
    }
}

fn wide_i128(value: i128) -> serde_json::Value {
    i64::try_from(value).map_or_else(
        |_| serde_json::json!(value.to_string()),
        |value| serde_json::json!(value),
    )
}

fn wide_u128(value: u128) -> serde_json::Value {
    u64::try_from(value).map_or_else(
        |_| serde_json::json!(value.to_string()),
        |value| serde_json::json!(value),
    )
}
