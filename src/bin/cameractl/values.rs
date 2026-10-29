//! Turn an agent's `--value` into the PTP value the camera advertised.

use sony_ptp3::catalog::{self, PropertyCode};
use sony_ptp3::datatype as dt;
use sony_ptp3::{DeviceProperty, PropForm, PropValue};

use crate::error::CliError;
use crate::names::slugify;
use crate::parse::code16;

pub fn datatype_name(code: u16) -> String {
    match code {
        0 => "none".to_owned(),
        dt::INT8 => "int8".to_owned(),
        dt::UINT8 => "uint8".to_owned(),
        dt::INT16 => "int16".to_owned(),
        dt::UINT16 => "uint16".to_owned(),
        dt::INT32 => "int32".to_owned(),
        dt::UINT32 => "uint32".to_owned(),
        dt::INT64 => "int64".to_owned(),
        dt::UINT64 => "uint64".to_owned(),
        dt::INT128 => "int128".to_owned(),
        dt::UINT128 => "uint128".to_owned(),
        dt::AINT8 => "int8-array".to_owned(),
        dt::AUINT8 => "uint8-array".to_owned(),
        dt::AINT16 => "int16-array".to_owned(),
        dt::AUINT16 => "uint16-array".to_owned(),
        dt::AINT32 => "int32-array".to_owned(),
        dt::AUINT32 => "uint32-array".to_owned(),
        dt::AINT64 => "int64-array".to_owned(),
        dt::AUINT64 => "uint64-array".to_owned(),
        dt::AINT128 => "int128-array".to_owned(),
        dt::AUINT128 => "uint128-array".to_owned(),
        dt::STR => "string".to_owned(),
        other => format!("{other:#06x}"),
    }
}

pub fn as_i128(value: &PropValue) -> Option<i128> {
    match value {
        PropValue::I8(value) => Some(i128::from(*value)),
        PropValue::U8(value) => Some(i128::from(*value)),
        PropValue::I16(value) => Some(i128::from(*value)),
        PropValue::U16(value) => Some(i128::from(*value)),
        PropValue::I32(value) => Some(i128::from(*value)),
        PropValue::U32(value) => Some(i128::from(*value)),
        PropValue::I64(value) => Some(i128::from(*value)),
        PropValue::U64(value) => Some(i128::from(*value)),
        PropValue::I128(value) => Some(*value),
        PropValue::U128(value) => i128::try_from(*value).ok(),
        _ => None,
    }
}

pub fn parse_property_value(property: &DeviceProperty, raw: &str) -> Result<PropValue, CliError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(CliError::usage(
            "value is empty",
            "Pass a number or a label from cameractl property get.",
        ));
    }
    if property.datatype == dt::STR {
        return Ok(PropValue::Str(raw.to_owned()));
    }
    if is_array(property.datatype) {
        return parse_array(property.datatype, raw);
    }
    let parsed = if looks_like_int(raw) {
        parse_scalar(property.datatype, raw)?
    } else if let Some(labeled) = match_label(property, raw)? {
        labeled
    } else {
        return Err(rejected(property, raw));
    };
    check_membership(property, &parsed, raw)?;
    Ok(parsed)
}

pub fn parse_scalar(datatype: u16, raw: &str) -> Result<PropValue, CliError> {
    let raw = raw.trim();
    match datatype {
        dt::STR => Ok(PropValue::Str(raw.to_owned())),
        dt::UINT128 => Ok(PropValue::U128(
            parse_u128(raw).map_err(|_| bad_fit(raw, datatype))?,
        )),
        dt::INT128 => Ok(PropValue::I128(
            parse_i128(raw).map_err(|_| bad_fit(raw, datatype))?,
        )),
        other if is_array(other) => parse_array(other, raw),
        other => {
            let value = parse_i128(raw).map_err(|_| bad_fit(raw, other))?;
            value_from_i128(other, value).ok_or_else(|| bad_fit(raw, other))
        }
    }
}

fn match_label(property: &DeviceProperty, raw: &str) -> Result<Option<PropValue>, CliError> {
    let want = slugify(raw);
    let mut hits = Vec::new();
    let mut push = |value: PropValue| {
        if !hits.contains(&value) {
            hits.push(value);
        }
    };
    for (number, label) in catalog::property_labels(PropertyCode(property.code)) {
        if label.eq_ignore_ascii_case(raw) || slugify(label) == want {
            if let Some(value) = value_from_i128(property.datatype, number) {
                push(value);
            }
        }
    }
    for choice in settable(property).iter().chain(readable(property)) {
        let named = choice
            .label_keys()
            .find_map(|key| catalog::value_label(PropertyCode(property.code), key));
        let display = choice.to_string();
        if display.eq_ignore_ascii_case(raw)
            || named.is_some_and(|label| label.eq_ignore_ascii_case(raw) || slugify(label) == want)
        {
            push(choice.clone());
        }
    }
    match hits.len() {
        0 => Ok(None),
        1 => Ok(hits.pop()),
        _ => Err(CliError::usage(
            format!(
                "value {raw} matches more than one {} value",
                property_name(property)
            ),
            format!(
                "Pass --code {} with a single numeric value.",
                code16(property.code)
            ),
        )
        .input(raw)),
    }
}

fn check_membership(
    property: &DeviceProperty,
    value: &PropValue,
    raw: &str,
) -> Result<(), CliError> {
    match &property.form {
        PropForm::Enumeration { set, get_set } => {
            let allowed: &[PropValue] = if !set.is_empty() { set } else { get_set };
            if !allowed.is_empty() && !allowed.contains(value) {
                return Err(rejected(property, raw));
            }
            Ok(())
        }
        PropForm::Range { min, max, .. } => {
            let (Some(value), Some(min), Some(max)) = (as_i128(value), as_i128(min), as_i128(max))
            else {
                return Ok(());
            };
            if value < min || value > max {
                return Err(CliError::usage(
                    format!(
                        "value {raw} is outside {min}..={max} for {}",
                        property_name(property)
                    ),
                    format!(
                        "Run: cameractl property get --code {}",
                        code16(property.code)
                    ),
                )
                .input(raw));
            }
            Ok(())
        }
        PropForm::None => Ok(()),
    }
}

fn rejected(property: &DeviceProperty, raw: &str) -> CliError {
    CliError::usage(
        format!(
            "value {raw} is not valid for {}. Accepted: {}",
            property_name(property),
            sample_values(property)
        ),
        format!(
            "Run: cameractl property get --code {} --format json",
            code16(property.code)
        ),
    )
    .input(raw)
}

fn sample_values(property: &DeviceProperty) -> String {
    let values = settable(property);
    if values.is_empty() {
        if let PropForm::Range { min, max, .. } = &property.form {
            return format!(
                "{}..={}",
                display(property.code, min),
                display(property.code, max)
            );
        }
        return format!("a {} value", datatype_name(property.datatype));
    }
    let mut parts: Vec<String> = values
        .iter()
        .take(12)
        .map(|value| display(property.code, value))
        .collect();
    if values.len() > 12 {
        parts.push(format!("{} more", values.len() - 12));
    }
    parts.join(", ")
}

fn display(code: u16, value: &PropValue) -> String {
    match value
        .label_keys()
        .find_map(|key| catalog::value_label(PropertyCode(code), key))
    {
        Some(label) => format!("{label} ({value})"),
        None => value.to_string(),
    }
}

fn property_name(property: &DeviceProperty) -> String {
    catalog::property(PropertyCode(property.code))
        .map(|spec| spec.name.to_owned())
        .unwrap_or_else(|| code16(property.code))
}

fn settable(property: &DeviceProperty) -> &[PropValue] {
    match &property.form {
        PropForm::Enumeration { set, get_set } => {
            if !set.is_empty() {
                set
            } else {
                get_set
            }
        }
        _ => &[],
    }
}

fn readable(property: &DeviceProperty) -> &[PropValue] {
    match &property.form {
        PropForm::Enumeration { get_set, .. } => get_set,
        _ => &[],
    }
}

fn value_from_i128(datatype: u16, value: i128) -> Option<PropValue> {
    match datatype {
        dt::INT8 => i8::try_from(value).ok().map(PropValue::I8),
        dt::UINT8 => u8::try_from(value).ok().map(PropValue::U8),
        dt::INT16 => i16::try_from(value).ok().map(PropValue::I16),
        dt::UINT16 => u16::try_from(value).ok().map(PropValue::U16),
        dt::INT32 => i32::try_from(value).ok().map(PropValue::I32),
        dt::UINT32 => u32::try_from(value).ok().map(PropValue::U32),
        dt::INT64 => i64::try_from(value).ok().map(PropValue::I64),
        dt::UINT64 => u64::try_from(value).ok().map(PropValue::U64),
        dt::INT128 => Some(PropValue::I128(value)),
        dt::UINT128 => u128::try_from(value).ok().map(PropValue::U128),
        _ => None,
    }
}

fn looks_like_int(raw: &str) -> bool {
    let digits = raw.strip_prefix('-').unwrap_or(raw);
    if digits.starts_with("0x") || digits.starts_with("0X") {
        return true;
    }
    !digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit())
}

fn bad_fit(raw: &str, datatype: u16) -> CliError {
    CliError::usage(
        format!("value {raw} does not fit {}", datatype_name(datatype)),
        format!("Pass a {} value.", datatype_name(datatype)),
    )
    .input(raw)
}

fn parse_i128(raw: &str) -> Result<i128, ()> {
    let (negative, digits) = match raw.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, raw),
    };
    let magnitude = parse_magnitude(digits)?;
    if negative {
        if magnitude == 1u128 << 127 {
            return Ok(i128::MIN);
        }
        let value = i128::try_from(magnitude).map_err(|_| ())?;
        value.checked_neg().ok_or(())
    } else {
        i128::try_from(magnitude).map_err(|_| ())
    }
}

fn parse_u128(raw: &str) -> Result<u128, ()> {
    if raw.starts_with('-') {
        return Err(());
    }
    parse_magnitude(raw)
}

fn parse_magnitude(digits: &str) -> Result<u128, ()> {
    if digits.is_empty() {
        return Err(());
    }
    if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        if hex.is_empty() || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(());
        }
        return u128::from_str_radix(hex, 16).map_err(|_| ());
    }
    if digits.chars().all(|ch| ch.is_ascii_digit()) {
        return digits.parse::<u128>().map_err(|_| ());
    }
    Err(())
}

fn is_array(datatype: u16) -> bool {
    matches!(
        datatype,
        dt::AINT8
            | dt::AUINT8
            | dt::AINT16
            | dt::AUINT16
            | dt::AINT32
            | dt::AUINT32
            | dt::AINT64
            | dt::AUINT64
            | dt::AINT128
            | dt::AUINT128
    )
}

fn parse_array(datatype: u16, raw: &str) -> Result<PropValue, CliError> {
    let parsed: serde_json::Value = serde_json::from_str(raw).map_err(|_| {
        CliError::usage(
            "array values must be JSON, for example [1, 2]",
            "Quote the flag so the shell keeps the brackets.",
        )
        .input(raw)
    })?;
    let serde_json::Value::Array(items) = parsed else {
        return Err(CliError::usage(
            "array values must be a JSON array",
            "Example: --value '[1, 2]'",
        )
        .input(raw));
    };
    if items.len() > 10_000 {
        return Err(CliError::usage(
            "array values are limited to 10000 items",
            "Pass a shorter array.",
        ));
    }
    match datatype {
        dt::AINT8 => Ok(PropValue::I8Array(collect(
            &items,
            dt::INT8,
            |value| match value {
                PropValue::I8(inner) => Some(inner),
                _ => None,
            },
        )?)),
        dt::AUINT8 => Ok(PropValue::U8Array(collect(
            &items,
            dt::UINT8,
            |value| match value {
                PropValue::U8(inner) => Some(inner),
                _ => None,
            },
        )?)),
        dt::AINT16 => Ok(PropValue::I16Array(collect(
            &items,
            dt::INT16,
            |value| match value {
                PropValue::I16(inner) => Some(inner),
                _ => None,
            },
        )?)),
        dt::AUINT16 => Ok(PropValue::U16Array(collect(
            &items,
            dt::UINT16,
            |value| match value {
                PropValue::U16(inner) => Some(inner),
                _ => None,
            },
        )?)),
        dt::AINT32 => Ok(PropValue::I32Array(collect(
            &items,
            dt::INT32,
            |value| match value {
                PropValue::I32(inner) => Some(inner),
                _ => None,
            },
        )?)),
        dt::AUINT32 => Ok(PropValue::U32Array(collect(
            &items,
            dt::UINT32,
            |value| match value {
                PropValue::U32(inner) => Some(inner),
                _ => None,
            },
        )?)),
        dt::AINT64 => Ok(PropValue::I64Array(collect(
            &items,
            dt::INT64,
            |value| match value {
                PropValue::I64(inner) => Some(inner),
                _ => None,
            },
        )?)),
        dt::AUINT64 => Ok(PropValue::U64Array(collect(
            &items,
            dt::UINT64,
            |value| match value {
                PropValue::U64(inner) => Some(inner),
                _ => None,
            },
        )?)),
        dt::AINT128 => Ok(PropValue::I128Array(collect(
            &items,
            dt::INT128,
            |value| match value {
                PropValue::I128(inner) => Some(inner),
                _ => None,
            },
        )?)),
        dt::AUINT128 => Ok(PropValue::U128Array(collect(
            &items,
            dt::UINT128,
            |value| match value {
                PropValue::U128(inner) => Some(inner),
                _ => None,
            },
        )?)),
        _ => Err(CliError::usage(
            format!("{} is not an array datatype", datatype_name(datatype)),
            "Pass a scalar --value for this property.",
        )),
    }
}

fn collect<T>(
    items: &[serde_json::Value],
    element: u16,
    take: impl Fn(PropValue) -> Option<T>,
) -> Result<Vec<T>, CliError> {
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let value = json_atom(element, item)?;
        let Some(inner) = take(value) else {
            return Err(CliError::usage(
                format!("an array item does not fit {}", datatype_name(element)),
                "Pass a JSON array of integers, for example [1, 2].",
            ));
        };
        out.push(inner);
    }
    Ok(out)
}

fn json_atom(datatype: u16, value: &serde_json::Value) -> Result<PropValue, CliError> {
    match value {
        serde_json::Value::Number(number) => {
            let Some(text) = integer_text(number) else {
                return Err(CliError::usage(
                    "array items must be integers",
                    "Pass a JSON array of integers, for example [1, 2].",
                ));
            };
            parse_scalar(datatype, &text)
        }
        serde_json::Value::String(text) => parse_scalar(datatype, text),
        _ => Err(CliError::usage(
            "array items must be integers",
            "Pass a JSON array of integers, for example [1, 2].",
        )),
    }
}

fn integer_text(number: &serde_json::Number) -> Option<String> {
    if let Some(value) = number.as_i64() {
        return Some(value.to_string());
    }
    number.as_u64().map(|value| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sony_ptp3::{EnableState, GetSet};

    fn property(code: u16, datatype: u16, form: PropForm, current: PropValue) -> DeviceProperty {
        DeviceProperty {
            code,
            datatype,
            get_set: GetSet::ReadWrite,
            enabled: EnableState::Enabled,
            factory: current.clone(),
            current,
            form,
        }
    }

    #[test]
    fn iso_accepts_a_number_or_a_catalog_label() {
        let property = property(0xd21e, dt::UINT32, PropForm::None, PropValue::U32(100));
        assert_eq!(
            parse_property_value(&property, "400").unwrap(),
            PropValue::U32(400)
        );
        assert_eq!(
            parse_property_value(&property, "ISO 400").unwrap(),
            PropValue::U32(400)
        );
    }

    #[test]
    fn a_label_outside_the_camera_enumeration_is_rejected() {
        let property = property(
            0xd21e,
            dt::UINT32,
            PropForm::Enumeration {
                set: vec![PropValue::U32(100)],
                get_set: Vec::new(),
            },
            PropValue::U32(100),
        );
        let error = parse_property_value(&property, "ISO 400").unwrap_err();
        assert_eq!(error.exit, 2);
        assert_eq!(error.input.as_deref(), Some("ISO 400"));
        assert!(error.message.contains("ISO 100"));
    }

    #[test]
    fn white_balance_accepts_awb() {
        let property = property(0x5005, dt::UINT16, PropForm::None, PropValue::U16(2));
        assert_eq!(
            parse_property_value(&property, "AWB").unwrap(),
            PropValue::U16(2)
        );
    }

    #[test]
    fn ranges_reject_values_outside_the_advertised_bounds() {
        let property = property(
            0x5000,
            dt::UINT16,
            PropForm::Range {
                min: PropValue::U16(1),
                max: PropValue::U16(10),
                step: PropValue::U16(1),
            },
            PropValue::U16(1),
        );
        assert!(parse_property_value(&property, "11").is_err());
        assert_eq!(
            parse_property_value(&property, "5").unwrap(),
            PropValue::U16(5)
        );
    }

    #[test]
    fn arrays_are_json() {
        let value = parse_scalar(dt::AUINT16, "[1, 2]").unwrap();
        assert_eq!(value, PropValue::U16Array(vec![1, 2]));
    }
}
