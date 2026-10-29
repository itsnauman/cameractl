//! Sony Camera Control PTP 3.
//!
//! Connect a camera over USB in PC Remote mode, then open a [`Camera`].
//! On macOS, Image Capture claims the device; [`ConnectOptions::release_system_client`]
//! stops that process so this library can use the PTP interface.
//!
//! [`catalog`] names every operation, property, control, and event in the
//! PTP 3 reference. A request is sent only when the connected camera
//! advertised that code. Anything else returns [`Error::Unsupported`].
//!
//! ```no_run
//! # fn main() -> Result<(), sony_ptp3::Error> {
//! let mut camera = sony_ptp3::Camera::try_new()?;
//! camera.shoot()?;
//! camera.close()?;
//! # Ok(())
//! # }
//! ```

#![deny(unsafe_code)]

mod camera;
pub mod catalog;
mod codec;
pub mod error;
mod prop_value;
mod transport;

pub use camera::{
    data_type, object, ButtonPosition, Camera, ConnectOptions, DisplayStringList, FunctionMode,
    UploadChunk, AFL_BUTTON, BUTTON_DOWN, BUTTON_UP, INITIATOR_VERSION,
};
pub use codec::{
    CallResult, DeviceInfo, DeviceProperty, EnableState, Event, ExtDeviceInfo, GetSet,
    LiveViewFrame, ObjectInfo, PropForm, StorageInfo,
};
pub use error::{Capability, Error};
pub use prop_value::PropValue;

/// PTP datatype codes accepted by [`PropValue::read`].
pub mod datatype {
    pub use crate::prop_value::{
        AINT128, AINT16, AINT32, AINT64, AINT8, AUINT128, AUINT16, AUINT32, AUINT64, AUINT8,
        INT128, INT16, INT32, INT64, INT8, STR, UINT128, UINT16, UINT32, UINT64, UINT8,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_covers_the_reference_for_any_model() {
        assert_eq!(
            catalog::operation(catalog::code::operation::GET_DEVICE_INFO).map(|spec| spec.name),
            Some("GetDeviceInfo")
        );
        assert_eq!(
            catalog::operation(catalog::code::operation::SDIO_GET_OSDIMAGE).map(|spec| spec.name),
            Some("SDIO_GetOSDImage")
        );
        assert_eq!(
            catalog::value_label(catalog::PropertyCode(0x5005), 2),
            Some("AWB")
        );
    }

    #[test]
    fn iso_labels_include_iso_400() {
        let labels: Vec<_> =
            catalog::property_labels(catalog::code::property::ISO_SENSITIVITY).collect();
        assert!(labels.contains(&(400, "ISO 400")));
    }

    #[test]
    fn unsupported_operation_names_the_camera() {
        let code = catalog::code::operation::SDIO_GET_OSDIMAGE.0;
        let error = Error::unsupported("ZV-E10", Capability::Operation, code);
        assert_eq!(
            error.to_string(),
            format!("ZV-E10 does not support operation SDIO_GetOSDImage ({code:#06x})")
        );
    }
}
