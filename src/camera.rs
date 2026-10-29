//! High-level Sony PTP 3 session.

use std::time::Duration;

use crate::catalog::{self, code, ControlCode, EventCode, OperationCode, PropertyCode};
use crate::codec::{
    self, CallResult, DeviceInfo, DeviceProperty, Event, ExtDeviceInfo, LiveViewFrame, ObjectInfo,
    StorageInfo,
};
use crate::error::{Capability, Error};
use crate::prop_value::{PropValue, UINT16};
use crate::transport::Session;

/// Function mode passed to `SDIO_OpenSession`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionMode {
    /// Remote control.
    RemoteControl = 0,
    /// Content transfer.
    ContentTransfer = 1,
    /// Remote control with content transfer.
    RemoteWithTransfer = 2,
}

/// Options for [`Camera::try_new_with`].
#[derive(Debug, Clone)]
pub struct ConnectOptions {
    /// `None` opens a standard session for remote control.
    /// `Some` closes that session and opens an SDIO session in this mode.
    pub function_mode: Option<FunctionMode>,
    /// Reset the USB device while opening the session.
    ///
    /// [`Camera::try_new_with`] also sets this after response `0x2004` and retries once.
    pub reset_device: bool,
    /// On macOS, Image Capture's `ptpcamerad` holds the camera. Releasing it
    /// is required before this process can claim the PTP interface.
    pub release_system_client: bool,
    /// USB timeout for the handshake and later transactions, until a method changes it.
    ///
    /// The default is five seconds.
    pub timeout: Duration,
}

/// Pressed or released position of a button control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonPosition {
    /// Released. The wire value is [`BUTTON_UP`].
    Up,
    /// Pressed. The wire value is [`BUTTON_DOWN`].
    Down,
}

impl ButtonPosition {
    fn wire(self) -> u16 {
        match self {
            Self::Up => BUTTON_UP,
            Self::Down => BUTTON_DOWN,
        }
    }
}

/// Whether an `SDIO_UploadPartialData` chunk ends the object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadChunk {
    /// Further chunks follow this one.
    More,
    /// This chunk is the last one.
    Last,
}

impl UploadChunk {
    fn wire(self) -> u32 {
        match self {
            Self::More => 0,
            Self::Last => 1,
        }
    }
}

/// Dataset from `SDIO_GetDisplayStringList`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayStringList {
    /// First response parameter. The camera uses `0` when it omits the parameter.
    pub parameter: u32,
    /// Raw string-list dataset.
    pub bytes: Vec<u8>,
}

impl Default for ConnectOptions {
    fn default() -> Self {
        Self {
            function_mode: None,
            reset_device: false,
            release_system_client: cfg!(target_os = "macos"),
            timeout: Duration::from_secs(5),
        }
    }
}

/// Initiator protocol version 3.00, as required by the PTP 3 handshake.
pub const INITIATOR_VERSION: u32 = 0x012c;

/// Live-view and post-capture object handles used by PTP 3.
pub mod object {
    pub const CAPTURED: u32 = 0xffff_c001;
    pub const LIVE_VIEW: u32 = 0xffff_c002;
    pub const SHOT_WITH_INFO: u32 = 0xffff_c003;
    pub const CAMERA_SETTING: u32 = 0xffff_c004;
    pub const FTP_SETTING: u32 = 0xffff_c005;
}

/// Released button position. Wire value `0x0001`.
pub const BUTTON_UP: u16 = 0x0001;
/// Pressed button position. Wire value `0x0002`.
pub const BUTTON_DOWN: u16 = 0x0002;

/// AFL, named by the control procedure in the PTP 3 reference.
pub const AFL_BUTTON: ControlCode = ControlCode(0xd2c4);

/// Upload and download type codes from the reference.
pub mod data_type {
    pub const CUBE_FILE: u32 = 0x0002_0001;
    pub const SCENE_FILE: u32 = 0x0007_0001;
}

/// A connected Sony PTP 3 camera.
///
/// Requests are checked against the operation, property, and control lists
/// this body returned during the handshake. A code it did not advertise
/// returns [`Error::Unsupported`] and is not sent.
///
/// [`Self::close`] ends the PTP session and returns the camera's response.
/// Dropping the value also closes the session, and discards that response.
pub struct Camera {
    session: Session,
    info: DeviceInfo,
    extended_info: ExtDeviceInfo,
    properties: Vec<DeviceProperty>,
}

impl Camera {
    /// Open the first Sony PTP camera in remote-control mode and complete the handshake.
    pub fn try_new() -> Result<Self, Error> {
        Self::try_new_with(ConnectOptions::default())
    }

    /// Open a session with `options`.
    ///
    /// On the first attempt, response `0x2004` (invalid transaction id) sets
    /// [`ConnectOptions::reset_device`] and tries once more. A second failure
    /// is returned as that response.
    pub fn try_new_with(mut options: ConnectOptions) -> Result<Self, Error> {
        for attempt in 0..2 {
            let session = Session::try_new(&options)?;
            match handshake(session, &options) {
                Ok(camera) => return Ok(camera),
                Err(Error::Response { code: 0x2004, .. }) if attempt == 0 => {
                    options.reset_device = true;
                }
                Err(error) => return Err(error),
            }
        }
        Err(Error::rejected(code::operation::OPEN_SESSION.0, 0x2004))
    }

    #[must_use]
    pub fn device_info(&self) -> &DeviceInfo {
        &self.info
    }

    #[must_use]
    pub fn ext_device_info(&self) -> &ExtDeviceInfo {
        &self.extended_info
    }

    /// Whether `GetDeviceInfo` listed this operation.
    #[must_use]
    pub fn supports_operation(&self, code: OperationCode) -> bool {
        self.info.operations.contains(&code.0)
    }

    /// Whether the handshake listed this device property.
    #[must_use]
    pub fn supports_property(&self, code: PropertyCode) -> bool {
        self.extended_info.properties.contains(&code.0)
            || self.info.device_properties.contains(&code.0)
            || self
                .properties
                .iter()
                .any(|property| property.code == code.0)
    }

    /// Whether `SDIO_GetExtDeviceInfo` listed this control.
    #[must_use]
    pub fn supports_control(&self, code: ControlCode) -> bool {
        self.extended_info.controls.contains(&code.0)
    }

    /// Whether `GetDeviceInfo` listed this event.
    #[must_use]
    pub fn supports_event(&self, code: EventCode) -> bool {
        self.info.events.contains(&code.0)
    }

    /// Properties from the most recent [`Self::refresh_properties`] call.
    #[must_use]
    pub fn properties(&self) -> &[DeviceProperty] {
        &self.properties
    }

    /// The cached property for `code`, from the last [`Self::refresh_properties`].
    #[must_use]
    pub fn property(&self, code: PropertyCode) -> Option<&DeviceProperty> {
        self.properties
            .iter()
            .find(|property| property.code == code.0)
    }

    pub fn refresh_properties(&mut self) -> Result<&[DeviceProperty], Error> {
        let result = self.transact_ok(
            code::operation::SDIO_GET_ALL_EXT_DEVICE_PROP_INFO,
            &[0, 1],
            None,
        )?;
        self.properties = codec::parse_device_properties(&result.data)?;
        Ok(&self.properties)
    }

    /// Set a property to `value`.
    ///
    /// Does not refresh [`Self::properties`]. Call [`Self::refresh_properties`]
    /// to read the camera's value back.
    pub fn set_property(&mut self, code: PropertyCode, value: &PropValue) -> Result<(), Error> {
        self.set_property_bytes(code, &value.write()?)
    }

    /// Set a property from its already-encoded PTP bytes.
    ///
    /// Does not refresh [`Self::properties`].
    pub fn set_property_bytes(&mut self, code: PropertyCode, value: &[u8]) -> Result<(), Error> {
        self.require_property(code)?;
        self.transact_ok(
            code::operation::SDIO_SET_EXT_DEVICE_PROP_VALUE,
            &[u32::from(code.0), 1],
            Some(value),
        )?;
        Ok(())
    }

    /// Send a control value.
    pub fn control(&mut self, code: ControlCode, value: &PropValue) -> Result<(), Error> {
        self.control_bytes(code, &value.write()?)
    }

    /// Send a control from its already-encoded PTP bytes.
    pub fn control_bytes(&mut self, code: ControlCode, value: &[u8]) -> Result<(), Error> {
        self.require_control(code)?;
        self.transact_ok(
            code::operation::SDIO_CONTROL_DEVICE,
            &[u32::from(code.0), 1],
            Some(value),
        )?;
        Ok(())
    }

    /// Send a button control in `position`.
    ///
    /// Returns [`Error::Datatype`] when the catalog lists a datatype other than
    /// none or [`UINT16`].
    pub fn button(&mut self, code: ControlCode, position: ButtonPosition) -> Result<(), Error> {
        self.require_control(code)?;
        if let Some(spec) = catalog::control(code) {
            if spec.datatype != 0 && spec.datatype != UINT16 {
                return Err(Error::Datatype(spec.datatype));
            }
        }
        let value = position.wire();
        self.control_bytes(code, &value.to_le_bytes())
    }

    /// Half-press, full-press, then release, in the order the reference requires.
    ///
    /// If the full-press fails, the releases still run. The full-press error is
    /// returned after those releases. A release error is returned only when the
    /// full-press succeeded.
    pub fn shoot(&mut self) -> Result<(), Error> {
        self.button(
            code::control::SHUTTER_HALF_RELEASE_S1_BUTTON,
            ButtonPosition::Down,
        )?;
        std::thread::sleep(Duration::from_millis(80));
        let pressed = self.button(
            code::control::SHUTTER_RELEASE_S2_BUTTON,
            ButtonPosition::Down,
        );
        std::thread::sleep(Duration::from_millis(120));
        let released = self.button(code::control::SHUTTER_RELEASE_S2_BUTTON, ButtonPosition::Up);
        std::thread::sleep(Duration::from_millis(40));
        let half = self.button(
            code::control::SHUTTER_HALF_RELEASE_S1_BUTTON,
            ButtonPosition::Up,
        );
        pressed?;
        released?;
        half
    }

    /// Zoom speed. Positive is tele, negative is wide, and zero stops.
    pub fn zoom(&mut self, speed: i8) -> Result<(), Error> {
        self.control_bytes(code::control::ZOOM_OPERATION, &speed.to_le_bytes())
    }

    /// Focus speed for the Near/Far control. Zero does not drive the lens.
    pub fn focus_near_far(&mut self, speed: i16) -> Result<(), Error> {
        self.control_bytes(code::control::NEAR_FAR, &speed.to_le_bytes())
    }

    /// Place the AF point. X is `0..=639` and Y is `0..=479`.
    pub fn set_af_position(&mut self, x: u16, y: u16) -> Result<(), Error> {
        let packed = (u32::from(x) << 16) | u32::from(y);
        self.control_bytes(code::control::AF_AREA_POSITION_X_Y, &packed.to_le_bytes())
    }

    /// Touch the live-view image. X is `0..=639` and Y is `0..=479`.
    pub fn remote_touch(&mut self, x: u16, y: u16) -> Result<(), Error> {
        let packed = (u32::from(x) << 16) | u32::from(y);
        self.control_bytes(
            code::control::REMOTE_TOUCH_OPERATION_X_Y,
            &packed.to_le_bytes(),
        )
    }

    /// Hold or release the movie button.
    pub fn hold_movie_button(&mut self, position: ButtonPosition) -> Result<(), Error> {
        self.button(code::control::MOVIE_REC_BUTTON_HOLD, position)
    }

    pub fn storage_ids(&mut self) -> Result<Vec<u32>, Error> {
        let result = self.transact_ok(code::operation::GET_STORAGE_IDS, &[], None)?;
        codec::parse_u32_list(&result.data)
    }

    pub fn storage_info(&mut self, storage_id: u32) -> Result<StorageInfo, Error> {
        let result = self.transact_ok(code::operation::GET_STORAGE_INFO, &[storage_id], None)?;
        codec::parse_storage_info(&result.data)
    }

    pub fn num_objects(&mut self, storage_id: u32, format: u32, parent: u32) -> Result<u32, Error> {
        let result = self.transact_ok(
            code::operation::GET_NUM_OBJECTS,
            &[storage_id, format, parent],
            None,
        )?;
        result
            .params
            .first()
            .copied()
            .ok_or(Error::MissingParameter {
                what: "GetNumObjects count",
            })
    }

    pub fn object_handles(
        &mut self,
        storage_id: u32,
        format: u32,
        parent: u32,
    ) -> Result<Vec<u32>, Error> {
        let result = self.transact_ok(
            code::operation::GET_OBJECT_HANDLES,
            &[storage_id, format, parent],
            None,
        )?;
        codec::parse_u32_list(&result.data)
    }

    pub fn object_info(&mut self, handle: u32) -> Result<ObjectInfo, Error> {
        let result = self.transact_ok(code::operation::GET_OBJECT_INFO, &[handle], None)?;
        codec::parse_object_info(&result.data)
    }

    pub fn object(&mut self, handle: u32) -> Result<Vec<u8>, Error> {
        self.with_timeout(Duration::from_secs(60), |camera| {
            let result = camera.transact_ok(code::operation::GET_OBJECT, &[handle], None)?;
            Ok(result.data)
        })
    }

    pub fn thumbnail(&mut self, handle: u32) -> Result<Vec<u8>, Error> {
        let result = self.transact_ok(code::operation::GET_THUMB, &[handle], None)?;
        Ok(result.data)
    }

    /// Read part of an object with the standard PTP operation.
    ///
    /// Some cameras reject card-file handles here. Those files are read with
    /// [`Self::partial_large_object`].
    pub fn partial_object(
        &mut self,
        handle: u32,
        offset: u32,
        max_bytes: u32,
    ) -> Result<Vec<u8>, Error> {
        let result = self.transact_ok(
            code::operation::GET_PARTIAL_OBJECT,
            &[handle, offset, max_bytes],
            None,
        )?;
        Ok(result.data)
    }

    /// Read part of a large object. `max_bytes` must not be `0xffffffff`.
    pub fn partial_large_object(
        &mut self,
        handle: u32,
        offset: u64,
        max_bytes: u32,
    ) -> Result<Vec<u8>, Error> {
        if max_bytes == u32::MAX {
            return Err(Error::PartialRange);
        }
        let result = self.transact_ok(
            code::operation::SDIO_GET_PARTIAL_LARGE_OBJECT,
            &[handle, offset as u32, (offset >> 32) as u32, max_bytes],
            None,
        )?;
        Ok(result.data)
    }

    pub fn send_object(&mut self, handle: u32, data: &[u8]) -> Result<(), Error> {
        self.with_timeout(Duration::from_secs(60), |camera| {
            camera.transact_ok(code::operation::SEND_OBJECT, &[handle], Some(data))?;
            Ok(())
        })
    }

    pub fn object_prop_value(&mut self, handle: u32, property: u16) -> Result<Vec<u8>, Error> {
        let result = self.transact_ok(
            code::operation::GET_OBJECT_PROP_VALUE,
            &[handle, u32::from(property)],
            None,
        )?;
        Ok(result.data)
    }

    pub fn object_prop_list(
        &mut self,
        handle: u32,
        format: u32,
        property: u32,
        group: u32,
        depth: u32,
    ) -> Result<Vec<u8>, Error> {
        let result = self.transact_ok(
            code::operation::GET_OBJECT_PROP_LIST,
            &[handle, format, property, group, depth],
            None,
        )?;
        Ok(result.data)
    }

    /// Read one live-view frame from object `0xffffc002`.
    ///
    /// Access denied (`0x200f`) is retried up to five times. Any other response
    /// is returned immediately.
    pub fn live_view_frame(&mut self) -> Result<LiveViewFrame, Error> {
        let mut last = None;
        for _ in 0..5 {
            let result = self.transact(code::operation::GET_OBJECT, &[object::LIVE_VIEW], None)?;
            if result.code == 0x200f {
                last = Some(result.code);
                std::thread::sleep(Duration::from_millis(40));
                continue;
            }
            if result.code != 0x2001 {
                return Err(Error::rejected(code::operation::GET_OBJECT.0, result.code));
            }
            return codec::parse_live_view(&result.data);
        }
        Err(Error::rejected(
            code::operation::GET_OBJECT.0,
            last.unwrap_or(0x200f),
        ))
    }

    pub fn captured_image(&mut self) -> Result<Vec<u8>, Error> {
        self.object(object::CAPTURED)
    }

    pub fn vendor_code_version(&mut self) -> Result<u32, Error> {
        let result = self.transact_ok(code::operation::SDIO_GET_VENDOR_CODE_VERSION, &[], None)?;
        result
            .params
            .first()
            .copied()
            .ok_or(Error::MissingParameter {
                what: "vendor code version",
            })
    }

    /// Read a display-string list.
    ///
    /// The parameter is `0` when the response omits it.
    pub fn display_string_list(&mut self, list_type: u32) -> Result<DisplayStringList, Error> {
        let result = self.transact_ok(
            code::operation::SDIO_GET_DISPLAY_STRING_LIST,
            &[list_type],
            None,
        )?;
        Ok(DisplayStringList {
            parameter: result.params.first().copied().unwrap_or(0),
            bytes: result.data,
        })
    }

    /// `kind` is `1` for feet and `2` for meters.
    pub fn lens_information(&mut self, kind: u32) -> Result<Vec<u8>, Error> {
        let result = self.transact_ok(code::operation::SDIO_GET_LENS_INFORMATION, &[kind], None)?;
        Ok(result.data)
    }

    pub fn operation_results_supported(&mut self) -> Result<Vec<u8>, Error> {
        let result =
            self.transact_ok(code::operation::SDIO_OPERATION_RESULTS_SUPPORTED, &[], None)?;
        Ok(result.data)
    }

    pub fn ftp_job_list(&mut self) -> Result<Vec<u8>, Error> {
        let result = self.transact_ok(code::operation::SDIO_GET_FTPJOB_LIST, &[0], None)?;
        Ok(result.data)
    }

    pub fn control_ftp_job_list(
        &mut self,
        control_type: u32,
        target: u32,
        dataset: &[u8],
    ) -> Result<(), Error> {
        self.transact_ok(
            code::operation::SDIO_CONTROL_FTPJOB_LIST,
            &[control_type, target],
            Some(dataset),
        )?;
        Ok(())
    }

    pub fn ftp_setting_list(&mut self) -> Result<Vec<u8>, Error> {
        let result = self.transact_ok(code::operation::SDIO_GET_FTPSETTING_LIST, &[], None)?;
        Ok(result.data)
    }

    pub fn set_ftp_setting_list(&mut self, dataset: &[u8]) -> Result<(), Error> {
        self.transact_ok(
            code::operation::SDIO_SET_FTPSETTING_LIST,
            &[],
            Some(dataset),
        )?;
        Ok(())
    }

    pub fn set_ftp_setting_file_password(
        &mut self,
        data_type: u32,
        password: &[u8],
    ) -> Result<(), Error> {
        self.transact_ok(
            code::operation::SDIO_SET_FTPSETTING_FILE_PASSWORD,
            &[data_type],
            Some(password),
        )?;
        Ok(())
    }

    pub fn set_contents_transfer_mode(
        &mut self,
        select: u32,
        mode: u32,
        extra: u32,
    ) -> Result<(), Error> {
        self.transact_ok(
            code::operation::SDIO_SET_CONTENTS_TRANSFER_MODE,
            &[select, mode, extra],
            None,
        )?;
        Ok(())
    }

    pub fn upload_data(&mut self, kind: u32, dataset: &[u8]) -> Result<(), Error> {
        self.with_timeout(Duration::from_secs(60), |camera| {
            camera.transact_ok(code::operation::SDIO_UPLOAD_DATA, &[kind], Some(dataset))?;
            Ok(())
        })
    }

    pub fn control_upload_data(&mut self, control_type: u32, dataset: &[u8]) -> Result<(), Error> {
        self.transact_ok(
            code::operation::SDIO_CONTROL_UPLOAD_DATA,
            &[control_type],
            Some(dataset),
        )?;
        Ok(())
    }

    pub fn download_data(
        &mut self,
        kind: u32,
        option1: u32,
        option2: u32,
        option3: u32,
        option4: u32,
    ) -> Result<Vec<u8>, Error> {
        let result = self.transact_ok(
            code::operation::SDIO_DOWNLOAD_DATA,
            &[kind, option1, option2, option3, option4],
            None,
        )?;
        Ok(result.data)
    }

    pub fn request_firmware_update_check(
        &mut self,
        version: u32,
        file_size: u64,
    ) -> Result<(), Error> {
        let mut dataset = Vec::with_capacity(12);
        dataset.extend_from_slice(&version.to_le_bytes());
        dataset.extend_from_slice(&file_size.to_le_bytes());
        self.transact_ok(
            code::operation::SDIO_REQUEST_FIRMWARE_UPDATE_CHECK,
            &[],
            Some(&dataset),
        )?;
        Ok(())
    }

    pub fn set_firmware_update_mode(&mut self, mode: u32, extra: u32) -> Result<(), Error> {
        self.transact_ok(
            code::operation::SDIO_SET_FIRMWARE_UPDATE_MODE,
            &[mode, extra],
            None,
        )?;
        Ok(())
    }

    /// Upload one chunk of a partial-data transfer.
    ///
    /// `position` says whether this chunk ends the object.
    pub fn upload_partial_data(
        &mut self,
        kind: u32,
        offset: u64,
        position: UploadChunk,
        data: &[u8],
    ) -> Result<(), Error> {
        self.with_timeout(Duration::from_secs(60), |camera| {
            camera.transact_ok(
                code::operation::SDIO_UPLOAD_PARTIAL_DATA,
                &[
                    kind,
                    offset as u32,
                    (offset >> 32) as u32,
                    data.len() as u32,
                    position.wire(),
                ],
                Some(data),
            )?;
            Ok(())
        })
    }

    /// Send a PTP operation this camera advertised. Device-busy responses are retried.
    ///
    /// Returns [`Error::Unsupported`] when GetDeviceInfo did not list `code`.
    /// Send a PTP operation this camera advertised. Device-busy responses are retried.
    ///
    /// Returns [`Error::Unsupported`] when `GetDeviceInfo` did not list `code`.
    pub fn call(
        &mut self,
        code: OperationCode,
        params: &[u32],
        data: Option<&[u8]>,
    ) -> Result<CallResult, Error> {
        self.transact(code, params, data)
    }

    /// [`Self::call`], returning an error unless the response is OK.
    pub fn call_ok(
        &mut self,
        code: OperationCode,
        params: &[u32],
        data: Option<&[u8]>,
    ) -> Result<CallResult, Error> {
        self.transact_ok(code, params, data)
    }

    /// Read one queued event, or wait up to `timeout` on the interrupt endpoint.
    ///
    /// Returns `Ok(None)` when the camera sends nothing before `timeout`, or
    /// when the device has no interrupt endpoint.
    pub fn poll_event(&mut self, timeout: Duration) -> Result<Option<Event>, Error> {
        self.session.poll_event(timeout)
    }

    /// End the PTP session.
    ///
    /// A second call is a no-op and returns `Ok(())`. Drop also closes the
    /// session, and ignores the response.
    pub fn close(&mut self) -> Result<(), Error> {
        self.session.close_session()
    }

    fn require_operation(&self, code: OperationCode) -> Result<(), Error> {
        ensure_supported(
            &self.info.model,
            Capability::Operation,
            &self.info.operations,
            code.0,
        )
    }

    fn require_property(&self, code: PropertyCode) -> Result<(), Error> {
        if self.supports_property(code) {
            Ok(())
        } else {
            Err(Error::unsupported(
                &self.info.model,
                Capability::Property,
                code.0,
            ))
        }
    }

    fn require_control(&self, code: ControlCode) -> Result<(), Error> {
        ensure_supported(
            &self.info.model,
            Capability::Control,
            &self.extended_info.controls,
            code.0,
        )
    }

    fn transact(
        &mut self,
        code: OperationCode,
        params: &[u32],
        data: Option<&[u8]>,
    ) -> Result<CallResult, Error> {
        self.require_operation(code)?;
        self.session.command(code.0, params, data)
    }

    fn transact_ok(
        &mut self,
        code: OperationCode,
        params: &[u32],
        data: Option<&[u8]>,
    ) -> Result<CallResult, Error> {
        self.require_operation(code)?;
        self.session.command_ok(code.0, params, data)
    }

    fn with_timeout<T>(
        &mut self,
        timeout: Duration,
        body: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let previous = self.session.timeout();
        self.session.set_timeout(timeout);
        let result = body(self);
        self.session.set_timeout(previous);
        result
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        let _ = self.session.close_session();
    }
}

fn handshake(mut session: Session, options: &ConnectOptions) -> Result<Camera, Error> {
    session.set_timeout(options.timeout);
    let info_result = session.command_ok(code::operation::GET_DEVICE_INFO.0, &[], None)?;
    let info = codec::parse_device_info(&info_result.data)?;
    for operation in [
        code::operation::OPEN_SESSION,
        code::operation::SDIO_CONNECT,
        code::operation::SDIO_GET_EXT_DEVICE_INFO,
        code::operation::SDIO_GET_ALL_EXT_DEVICE_PROP_INFO,
    ] {
        ensure_supported(
            &info.model,
            Capability::Operation,
            &info.operations,
            operation.0,
        )?;
    }
    if options.function_mode.is_some() {
        for operation in [
            code::operation::CLOSE_SESSION,
            code::operation::SDIO_OPEN_SESSION,
        ] {
            ensure_supported(
                &info.model,
                Capability::Operation,
                &info.operations,
                operation.0,
            )?;
        }
    }
    let opened = session.command(code::operation::OPEN_SESSION.0, &[1], None)?;
    accept_open(&opened, code::operation::OPEN_SESSION.0)?;
    if let Some(mode) = options.function_mode {
        session.close_ptp_session()?;
        accept_open(
            &session.command(
                code::operation::SDIO_OPEN_SESSION.0,
                &[1, mode as u32],
                None,
            )?,
            code::operation::SDIO_OPEN_SESSION.0,
        )?;
    }
    session.command_ok(code::operation::SDIO_CONNECT.0, &[1, 0, 0], None)?;
    session.command_ok(code::operation::SDIO_CONNECT.0, &[2, 0, 0], None)?;
    let ext_result = session.command_ok(
        code::operation::SDIO_GET_EXT_DEVICE_INFO.0,
        &[INITIATOR_VERSION, 1],
        None,
    )?;
    let vendor_version = ext_result.params.first().copied().unwrap_or(0);
    let ext = codec::parse_ext_device_info(&ext_result.data, vendor_version)?;
    session.command_ok(code::operation::SDIO_CONNECT.0, &[3, 0, 0], None)?;
    let mut camera = Camera {
        session,
        info,
        extended_info: ext,
        properties: Vec::new(),
    };
    camera.refresh_properties()?;
    Ok(camera)
}

fn ensure_supported(
    model: &str,
    kind: Capability,
    advertised: &[u16],
    code: u16,
) -> Result<(), Error> {
    if advertised.contains(&code) {
        Ok(())
    } else {
        Err(Error::unsupported(model, kind, code))
    }
}

fn accept_open(result: &CallResult, operation: u16) -> Result<(), Error> {
    if result.code == 0x2001 || result.code == 0x201e {
        Ok(())
    } else {
        Err(Error::rejected(operation, result.code))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unadvertised_operation_is_rejected() {
        let error =
            ensure_supported("ZV-E10", Capability::Operation, &[0x1001], 0x9238).unwrap_err();
        assert!(matches!(
            error,
            Error::Unsupported {
                code: 0x9238,
                kind: Capability::Operation,
                ..
            }
        ));
        assert!(ensure_supported("ZV-E10", Capability::Operation, &[0x1001], 0x1001).is_ok());
    }
}
