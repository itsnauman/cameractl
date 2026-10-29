//! Dataset decoders for PTP and Sony SDIO payloads.

use crate::catalog::{self, EventCode, PropertyCode};
use crate::error::Error;
#[cfg(test)]
use crate::prop_value::write_value;
use crate::prop_value::{PropValue, Reader};

/// Standard PTP device information returned by `GetDeviceInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub standard_version: u16,
    pub vendor_extension_id: u32,
    pub vendor_extension_version: u16,
    pub vendor_extension_desc: String,
    pub functional_mode: u16,
    pub operations: Vec<u16>,
    pub events: Vec<u16>,
    pub device_properties: Vec<u16>,
    pub capture_formats: Vec<u16>,
    pub image_formats: Vec<u16>,
    pub manufacturer: String,
    pub model: String,
    pub device_version: String,
    pub serial_number: String,
}

/// Property and control codes the camera reported for this session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtDeviceInfo {
    pub extension_version: u16,
    pub vendor_code_version: u32,
    pub properties: Vec<u16>,
    pub controls: Vec<u16>,
}

/// Whether the initiator may change a property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetSet {
    ReadOnly,
    ReadWrite,
    Other(u8),
}

/// Whether the camera is currently offering the property in its UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnableState {
    Disabled,
    Enabled,
    DisplayOnly,
    Other(u8),
}

/// Legal values advertised with a property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropForm {
    None,
    Range {
        min: PropValue,
        max: PropValue,
        step: PropValue,
    },
    Enumeration {
        set: Vec<PropValue>,
        get_set: Vec<PropValue>,
    },
}

/// One device property from `SDIO_GetAllExtDevicePropInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceProperty {
    pub code: u16,
    pub datatype: u16,
    pub get_set: GetSet,
    pub enabled: EnableState,
    pub factory: PropValue,
    pub current: PropValue,
    pub form: PropForm,
}

impl DeviceProperty {
    #[must_use]
    pub fn name(&self) -> &'static str {
        catalog::property(PropertyCode(self.code))
            .map(|spec| spec.name)
            .unwrap_or("Unknown")
    }

    #[must_use]
    pub fn current_label(&self) -> Option<&'static str> {
        self.current
            .label_keys()
            .find_map(|key| catalog::value_label(PropertyCode(self.code), key))
    }

    #[must_use]
    pub fn is_writable(&self) -> bool {
        self.get_set == GetSet::ReadWrite && self.enabled == EnableState::Enabled
    }
}

/// PTP storage descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageInfo {
    pub storage_type: u16,
    pub filesystem_type: u16,
    pub access: u16,
    pub max_capacity: u64,
    pub free_bytes: u64,
    pub free_images: u32,
    pub description: String,
    pub volume_label: String,
}

/// PTP object descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectInfo {
    pub storage_id: u32,
    pub format: u16,
    pub protection: u16,
    pub compressed_size: u32,
    pub thumb_format: u16,
    pub thumb_size: u32,
    pub thumb_width: u32,
    pub thumb_height: u32,
    pub image_width: u32,
    pub image_height: u32,
    pub image_bit_depth: u32,
    pub parent: u32,
    pub association_type: u16,
    pub association_desc: u32,
    pub sequence_number: u32,
    pub filename: String,
    pub capture_date: String,
    pub modification_date: String,
    pub keywords: String,
}

/// One live-view frame from object handle `0xffffc002`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveViewFrame {
    pub jpeg: Vec<u8>,
    pub focal_frame: Vec<u8>,
}

/// An event read from the camera's interrupt endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub code: u16,
    pub params: Vec<u32>,
}

impl Event {
    #[must_use]
    pub fn name(&self) -> Option<&'static str> {
        catalog::event(EventCode(self.code)).map(|spec| spec.name)
    }
}

/// Result of one PTP transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallResult {
    pub code: u16,
    pub params: Vec<u32>,
    pub data: Vec<u8>,
}

impl CallResult {
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.code == 0x2001
    }

    #[must_use]
    pub fn name(&self) -> &'static str {
        crate::error::response_name(self.code)
    }
}

pub fn parse_device_info(data: &[u8]) -> Result<DeviceInfo, Error> {
    let mut reader = Reader::new(data);
    let info = DeviceInfo {
        standard_version: reader.u16()?,
        vendor_extension_id: reader.u32()?,
        vendor_extension_version: reader.u16()?,
        vendor_extension_desc: take_string(&mut reader)?,
        functional_mode: reader.u16()?,
        operations: read_code_array(&mut reader)?,
        events: read_code_array(&mut reader)?,
        device_properties: read_code_array(&mut reader)?,
        capture_formats: read_code_array(&mut reader)?,
        image_formats: read_code_array(&mut reader)?,
        manufacturer: take_string(&mut reader)?,
        model: take_string(&mut reader)?,
        device_version: take_string(&mut reader)?,
        serial_number: take_string(&mut reader)?,
    };
    if reader.remaining() != 0 {
        return Err(Error::TrailingBytes {
            what: "device info",
            len: reader.remaining(),
        });
    }
    Ok(info)
}

pub fn parse_ext_device_info(
    data: &[u8],
    vendor_code_version: u32,
) -> Result<ExtDeviceInfo, Error> {
    let mut reader = Reader::new(data);
    let info = ExtDeviceInfo {
        extension_version: reader.u16()?,
        vendor_code_version,
        properties: read_code_array(&mut reader)?,
        controls: read_code_array(&mut reader)?,
    };
    if reader.remaining() != 0 {
        return Err(Error::TrailingBytes {
            what: "ext device info",
            len: reader.remaining(),
        });
    }
    Ok(info)
}

pub fn parse_device_properties(data: &[u8]) -> Result<Vec<DeviceProperty>, Error> {
    let mut reader = Reader::new(data);
    let count = reader.u64()?;
    if count > 10_000 {
        return Err(Error::InvalidLength {
            what: "property",
            len: count,
        });
    }
    let mut properties = Vec::with_capacity(count as usize);
    for _ in 0..count {
        properties.push(read_property(&mut reader)?);
    }
    if reader.remaining() != 0 {
        return Err(Error::TrailingBytes {
            what: "property array",
            len: reader.remaining(),
        });
    }
    Ok(properties)
}

#[cfg(test)]
pub fn write_device_properties(properties: &[DeviceProperty]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    out.extend_from_slice(&(properties.len() as u64).to_le_bytes());
    for property in properties {
        write_property(&mut out, property)?;
    }
    Ok(out)
}

pub fn parse_storage_info(data: &[u8]) -> Result<StorageInfo, Error> {
    let mut reader = Reader::new(data);
    Ok(StorageInfo {
        storage_type: reader.u16()?,
        filesystem_type: reader.u16()?,
        access: reader.u16()?,
        max_capacity: reader.u64()?,
        free_bytes: reader.u64()?,
        free_images: reader.u32()?,
        description: take_string(&mut reader)?,
        volume_label: take_string(&mut reader)?,
    })
}

pub fn parse_object_info(data: &[u8]) -> Result<ObjectInfo, Error> {
    let mut reader = Reader::new(data);
    Ok(ObjectInfo {
        storage_id: reader.u32()?,
        format: reader.u16()?,
        protection: reader.u16()?,
        compressed_size: reader.u32()?,
        thumb_format: reader.u16()?,
        thumb_size: reader.u32()?,
        thumb_width: reader.u32()?,
        thumb_height: reader.u32()?,
        image_width: reader.u32()?,
        image_height: reader.u32()?,
        image_bit_depth: reader.u32()?,
        parent: reader.u32()?,
        association_type: reader.u16()?,
        association_desc: reader.u32()?,
        sequence_number: reader.u32()?,
        filename: take_string(&mut reader)?,
        capture_date: take_string(&mut reader)?,
        modification_date: take_string(&mut reader)?,
        keywords: take_string(&mut reader)?,
    })
}

pub fn parse_u32_list(data: &[u8]) -> Result<Vec<u32>, Error> {
    if data.len() < 4 || !(data.len() - 4).is_multiple_of(4) {
        return Err(Error::Uint32Array {
            len: data.len(),
            prefix: data[..data.len().min(32)].to_vec(),
        });
    }
    let count_bytes: [u8; 4] = data[0..4].try_into().map_err(|_| Error::Truncated)?;
    let count = u32::from_le_bytes(count_bytes) as usize;
    if data.len() != 4 + count * 4 {
        return Err(Error::Uint32Count {
            count,
            len: data.len(),
        });
    }
    data[4..]
        .chunks_exact(4)
        .map(|chunk| {
            let bytes: [u8; 4] = chunk.try_into().map_err(|_| Error::Truncated)?;
            Ok(u32::from_le_bytes(bytes))
        })
        .collect()
}

pub fn parse_live_view(data: &[u8]) -> Result<LiveViewFrame, Error> {
    if data.len() < 16 {
        return Err(Error::LiveViewHeader);
    }
    let image_offset = le_u32(data, 0)? as usize;
    let image_size = le_u32(data, 4)? as usize;
    let frame_offset = le_u32(data, 8)? as usize;
    let frame_size = le_u32(data, 12)? as usize;
    let jpeg = slice_at(data, image_offset, image_size, "live view image")?;
    let focal_frame = slice_at(data, frame_offset, frame_size, "focal frame")?;
    Ok(LiveViewFrame {
        jpeg: jpeg.to_vec(),
        focal_frame: focal_frame.to_vec(),
    })
}

fn le_u32(data: &[u8], offset: usize) -> Result<u32, Error> {
    let Some(bytes) = data.get(offset..offset + 4) else {
        return Err(Error::Truncated);
    };
    let bytes: [u8; 4] = bytes.try_into().map_err(|_| Error::Truncated)?;
    Ok(u32::from_le_bytes(bytes))
}

fn slice_at<'a>(
    data: &'a [u8],
    offset: usize,
    size: usize,
    what: &'static str,
) -> Result<&'a [u8], Error> {
    let Some(end) = offset.checked_add(size) else {
        return Err(Error::OffsetOverflow { what });
    };
    data.get(offset..end).ok_or(Error::OutOfBounds { what })
}

fn read_code_array(reader: &mut Reader<'_>) -> Result<Vec<u16>, Error> {
    let count = reader.u32()?;
    if count > 10_000 {
        return Err(Error::InvalidLength {
            what: "code array",
            len: u64::from(count),
        });
    }
    let count = usize::try_from(count).map_err(|_| Error::InvalidLength {
        what: "code array",
        len: u64::from(count),
    })?;
    let mut codes = Vec::with_capacity(count);
    for _ in 0..count {
        codes.push(reader.u16()?);
    }
    Ok(codes)
}

fn take_string(reader: &mut Reader<'_>) -> Result<String, Error> {
    match reader.value(crate::prop_value::STR)? {
        PropValue::Str(value) => Ok(value),
        _ => Err(Error::ExpectedString),
    }
}

fn read_property(reader: &mut Reader<'_>) -> Result<DeviceProperty, Error> {
    let code = reader.u16()?;
    let datatype = reader.u16()?;
    let get_set = match reader.u8()? {
        0x00 => GetSet::ReadOnly,
        0x01 => GetSet::ReadWrite,
        other => GetSet::Other(other),
    };
    let enabled = match reader.u8()? {
        0x00 => EnableState::Disabled,
        0x01 => EnableState::Enabled,
        0x02 => EnableState::DisplayOnly,
        other => EnableState::Other(other),
    };
    let factory = reader.value(datatype)?;
    let current = reader.value(datatype)?;
    let form = match reader.u8()? {
        0x00 => PropForm::None,
        0x01 => PropForm::Range {
            min: reader.value(datatype)?,
            max: reader.value(datatype)?,
            step: reader.value(datatype)?,
        },
        0x02 => PropForm::Enumeration {
            set: read_value_list(reader, datatype)?,
            get_set: read_value_list(reader, datatype)?,
        },
        other => {
            return Err(Error::PropertyForm { code, flag: other });
        }
    };
    Ok(DeviceProperty {
        code,
        datatype,
        get_set,
        enabled,
        factory,
        current,
        form,
    })
}

fn read_value_list(reader: &mut Reader<'_>, datatype: u16) -> Result<Vec<PropValue>, Error> {
    let count = reader.u16()?;
    if u32::from(count) > 10_000 {
        return Err(Error::InvalidLength {
            what: "enumeration",
            len: u64::from(count),
        });
    }
    let count = usize::from(count);
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(reader.value(datatype)?);
    }
    Ok(values)
}

#[cfg(test)]
fn write_property(out: &mut Vec<u8>, property: &DeviceProperty) -> Result<(), Error> {
    out.extend_from_slice(&property.code.to_le_bytes());
    out.extend_from_slice(&property.datatype.to_le_bytes());
    out.push(match property.get_set {
        GetSet::ReadOnly => 0,
        GetSet::ReadWrite => 1,
        GetSet::Other(value) => value,
    });
    out.push(match property.enabled {
        EnableState::Disabled => 0,
        EnableState::Enabled => 1,
        EnableState::DisplayOnly => 2,
        EnableState::Other(value) => value,
    });
    write_value(out, &property.factory)?;
    write_value(out, &property.current)?;
    match &property.form {
        PropForm::None => out.push(0),
        PropForm::Range { min, max, step } => {
            out.push(1);
            write_value(out, min)?;
            write_value(out, max)?;
            write_value(out, step)?;
        }
        PropForm::Enumeration { set, get_set } => {
            out.push(2);
            write_value_list(out, set)?;
            write_value_list(out, get_set)?;
        }
    }
    Ok(())
}

#[cfg(test)]
fn write_value_list(out: &mut Vec<u8>, values: &[PropValue]) -> Result<(), Error> {
    out.extend_from_slice(&(values.len() as u16).to_le_bytes());
    for value in values {
        write_value(out, value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_info_matches_the_connected_zv1m2() {
        let info = parse_device_info(include_bytes!("../tests/fixtures/deviceinfo.bin")).unwrap();
        assert_eq!(info.model, "ZV-1M2");
        assert_eq!(info.manufacturer, "Sony Corporation");
        assert_eq!(info.device_version, "2.01");
        assert!(info.operations.contains(&0x1001));
        assert!(info.operations.contains(&0x9201));
    }

    #[test]
    fn ext_device_info_lists_properties_and_controls() {
        let info = parse_ext_device_info(
            include_bytes!("../tests/fixtures/extdeviceinfo.bin"),
            0x0136,
        )
        .unwrap();
        assert_eq!(info.extension_version, 0x012c);
        assert_eq!(info.properties.len(), 108);
        assert!(info.properties.contains(&0x5005));
        assert!(info.controls.contains(&0xd2c1));
    }

    #[test]
    fn property_dataset_round_trips() {
        let original = include_bytes!("../tests/fixtures/getall.bin");
        let properties = parse_device_properties(original).unwrap();
        assert_eq!(properties.len(), 108);
        let white_balance = properties
            .iter()
            .find(|property| property.code == 0x5005)
            .unwrap();
        assert_eq!(white_balance.current, PropValue::U16(0x0002));
        assert_eq!(white_balance.current_label(), Some("AWB"));
        let encoded = write_device_properties(&properties).unwrap();
        assert_eq!(encoded, original);
    }
}
