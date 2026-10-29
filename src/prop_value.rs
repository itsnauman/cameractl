//! PTP datatypes and the values carried in device properties and controls.

use std::fmt::{Display, Formatter};

use crate::error::Error;

pub const INT8: u16 = 0x0001;
pub const UINT8: u16 = 0x0002;
pub const INT16: u16 = 0x0003;
pub const UINT16: u16 = 0x0004;
pub const INT32: u16 = 0x0005;
pub const UINT32: u16 = 0x0006;
pub const INT64: u16 = 0x0007;
pub const UINT64: u16 = 0x0008;
pub const INT128: u16 = 0x0009;
pub const UINT128: u16 = 0x000a;
pub const AINT8: u16 = 0x4001;
pub const AUINT8: u16 = 0x4002;
pub const AINT16: u16 = 0x4003;
pub const AUINT16: u16 = 0x4004;
pub const AINT32: u16 = 0x4005;
pub const AUINT32: u16 = 0x4006;
pub const AINT64: u16 = 0x4007;
pub const AUINT64: u16 = 0x4008;
pub const AINT128: u16 = 0x4009;
pub const AUINT128: u16 = 0x400a;
pub const STR: u16 = 0xffff;

/// A decoded PTP value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropValue {
    I8(i8),
    U8(u8),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    I128(i128),
    U128(u128),
    Str(String),
    I8Array(Vec<i8>),
    U8Array(Vec<u8>),
    I16Array(Vec<i16>),
    U16Array(Vec<u16>),
    I32Array(Vec<i32>),
    U32Array(Vec<u32>),
    I64Array(Vec<i64>),
    U64Array(Vec<u64>),
    I128Array(Vec<i128>),
    U128Array(Vec<u128>),
}

impl PropValue {
    pub fn read(datatype: u16, data: &[u8]) -> Result<(Self, usize), Error> {
        let mut reader = Reader { data, index: 0 };
        let value = reader.value(datatype)?;
        Ok((value, reader.index))
    }

    pub fn write(&self) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        write_value(&mut out, self)?;
        Ok(out)
    }

    /// Numeric keys used to look up a documented enumeration label.
    ///
    /// Signed values yield both the signed reading and the same bits read as
    /// unsigned, because reference tables use either form.
    pub fn label_keys(&self) -> impl Iterator<Item = i128> {
        let keys = match *self {
            Self::I8(value) => [Some(i128::from(value)), Some(i128::from(value as u8))],
            Self::U8(value) => [Some(i128::from(value)), None],
            Self::I16(value) => [Some(i128::from(value)), Some(i128::from(value as u16))],
            Self::U16(value) => [Some(i128::from(value)), None],
            Self::I32(value) => [Some(i128::from(value)), Some(i128::from(value as u32))],
            Self::U32(value) => [Some(i128::from(value)), None],
            Self::I64(value) => [Some(i128::from(value)), Some(i128::from(value as u64))],
            Self::U64(value) => [Some(i128::from(value)), None],
            Self::I128(value) => [Some(value), None],
            Self::U128(value) => [i128::try_from(value).ok(), None],
            _ => [None, None],
        };
        keys.into_iter().flatten()
    }
}

impl Display for PropValue {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::I8(value) => write!(formatter, "{value}"),
            Self::U8(value) => write!(formatter, "{value}"),
            Self::I16(value) => write!(formatter, "{value}"),
            Self::U16(value) => write!(formatter, "{value}"),
            Self::I32(value) => write!(formatter, "{value}"),
            Self::U32(value) => write!(formatter, "{value}"),
            Self::I64(value) => write!(formatter, "{value}"),
            Self::U64(value) => write!(formatter, "{value}"),
            Self::I128(value) => write!(formatter, "{value}"),
            Self::U128(value) => write!(formatter, "{value}"),
            Self::Str(value) => write!(formatter, "{value}"),
            Self::I8Array(value) => write!(formatter, "{value:?}"),
            Self::U8Array(value) => write!(formatter, "{value:?}"),
            Self::I16Array(value) => write!(formatter, "{value:?}"),
            Self::U16Array(value) => write!(formatter, "{value:?}"),
            Self::I32Array(value) => write!(formatter, "{value:?}"),
            Self::U32Array(value) => write!(formatter, "{value:?}"),
            Self::I64Array(value) => write!(formatter, "{value:?}"),
            Self::U64Array(value) => write!(formatter, "{value:?}"),
            Self::I128Array(value) => write!(formatter, "{value:?}"),
            Self::U128Array(value) => write!(formatter, "{value:?}"),
        }
    }
}

pub(crate) struct Reader<'a> {
    data: &'a [u8],
    index: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, index: 0 }
    }

    pub(crate) fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.index)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, Error> {
        let byte = *self.data.get(self.index).ok_or(Error::Truncated)?;
        self.index += 1;
        Ok(byte)
    }

    pub(crate) fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.fixed()?))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.fixed()?))
    }

    pub(crate) fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_le_bytes(self.fixed()?))
    }

    pub(crate) fn bytes(&mut self, len: usize) -> Result<&'a [u8], Error> {
        let end = self.index.checked_add(len).ok_or(Error::Truncated)?;
        let bytes = self.data.get(self.index..end).ok_or(Error::Truncated)?;
        self.index = end;
        Ok(bytes)
    }

    pub(crate) fn value(&mut self, datatype: u16) -> Result<PropValue, Error> {
        Ok(match datatype {
            INT8 => PropValue::I8(self.u8()? as i8),
            UINT8 => PropValue::U8(self.u8()?),
            INT16 => PropValue::I16(self.u16()? as i16),
            UINT16 => PropValue::U16(self.u16()?),
            INT32 => PropValue::I32(self.u32()? as i32),
            UINT32 => PropValue::U32(self.u32()?),
            INT64 => PropValue::I64(self.u64()? as i64),
            UINT64 => PropValue::U64(self.u64()?),
            INT128 => PropValue::I128(i128::from_le_bytes(self.fixed()?)),
            UINT128 => PropValue::U128(u128::from_le_bytes(self.fixed()?)),
            STR => PropValue::Str(self.string()?),
            AINT8 => PropValue::I8Array(self.array(|reader| Ok(reader.u8()? as i8))?),
            AUINT8 => PropValue::U8Array(self.array(|reader| reader.u8())?),
            AINT16 => PropValue::I16Array(self.array(|reader| Ok(reader.u16()? as i16))?),
            AUINT16 => PropValue::U16Array(self.array(|reader| reader.u16())?),
            AINT32 => PropValue::I32Array(self.array(|reader| Ok(reader.u32()? as i32))?),
            AUINT32 => PropValue::U32Array(self.array(|reader| reader.u32())?),
            AINT64 => PropValue::I64Array(self.array(|reader| Ok(reader.u64()? as i64))?),
            AUINT64 => PropValue::U64Array(self.array(|reader| reader.u64())?),
            AINT128 => {
                PropValue::I128Array(self.array(|reader| Ok(i128::from_le_bytes(reader.fixed()?)))?)
            }
            AUINT128 => {
                PropValue::U128Array(self.array(|reader| Ok(u128::from_le_bytes(reader.fixed()?)))?)
            }
            other => return Err(Error::Datatype(other)),
        })
    }

    fn fixed<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let bytes = self.bytes(N)?;
        let mut array = [0; N];
        array.copy_from_slice(bytes);
        Ok(array)
    }

    fn array<T>(
        &mut self,
        mut read: impl FnMut(&mut Self) -> Result<T, Error>,
    ) -> Result<Vec<T>, Error> {
        let count = self.u32()?;
        if count > 1_000_000 {
            return Err(Error::InvalidLength {
                what: "array",
                len: u64::from(count),
            });
        }
        let count = count as usize;
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(read(self)?);
        }
        Ok(values)
    }

    fn string(&mut self) -> Result<String, Error> {
        let count = self.u8()? as usize;
        let raw = self.bytes(count * 2)?;
        let units = raw
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();
        let text = String::from_utf16(&units).map_err(|_| Error::InvalidString)?;
        Ok(text.trim_end_matches('\0').to_string())
    }
}

pub(crate) fn write_value(out: &mut Vec<u8>, value: &PropValue) -> Result<(), Error> {
    match value {
        PropValue::I8(value) => out.push(*value as u8),
        PropValue::U8(value) => out.push(*value),
        PropValue::I16(value) => out.extend_from_slice(&value.to_le_bytes()),
        PropValue::U16(value) => out.extend_from_slice(&value.to_le_bytes()),
        PropValue::I32(value) => out.extend_from_slice(&value.to_le_bytes()),
        PropValue::U32(value) => out.extend_from_slice(&value.to_le_bytes()),
        PropValue::I64(value) => out.extend_from_slice(&value.to_le_bytes()),
        PropValue::U64(value) => out.extend_from_slice(&value.to_le_bytes()),
        PropValue::I128(value) => out.extend_from_slice(&value.to_le_bytes()),
        PropValue::U128(value) => out.extend_from_slice(&value.to_le_bytes()),
        PropValue::Str(value) => write_string(out, value)?,
        PropValue::I8Array(values) => write_array(out, values, |out, value| out.push(*value as u8)),
        PropValue::U8Array(values) => write_array(out, values, |out, value| out.push(*value)),
        PropValue::I16Array(values) => write_array(out, values, |out, value| {
            out.extend_from_slice(&value.to_le_bytes())
        }),
        PropValue::U16Array(values) => write_array(out, values, |out, value| {
            out.extend_from_slice(&value.to_le_bytes())
        }),
        PropValue::I32Array(values) => write_array(out, values, |out, value| {
            out.extend_from_slice(&value.to_le_bytes())
        }),
        PropValue::U32Array(values) => write_array(out, values, |out, value| {
            out.extend_from_slice(&value.to_le_bytes())
        }),
        PropValue::I64Array(values) => write_array(out, values, |out, value| {
            out.extend_from_slice(&value.to_le_bytes())
        }),
        PropValue::U64Array(values) => write_array(out, values, |out, value| {
            out.extend_from_slice(&value.to_le_bytes())
        }),
        PropValue::I128Array(values) => write_array(out, values, |out, value| {
            out.extend_from_slice(&value.to_le_bytes())
        }),
        PropValue::U128Array(values) => write_array(out, values, |out, value| {
            out.extend_from_slice(&value.to_le_bytes())
        }),
    }
    Ok(())
}

fn write_array<T>(out: &mut Vec<u8>, values: &[T], mut write: impl FnMut(&mut Vec<u8>, &T)) {
    out.extend_from_slice(&(values.len() as u32).to_le_bytes());
    for value in values {
        write(out, value);
    }
}

fn write_string(out: &mut Vec<u8>, value: &str) -> Result<(), Error> {
    let mut units: Vec<u16> = value.encode_utf16().collect();
    if !units.is_empty() {
        units.push(0);
    }
    if units.len() > 255 {
        return Err(Error::StringTooLong);
    }
    out.push(units.len() as u8);
    for unit in units {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(())
}
