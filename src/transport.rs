//! USB still-image transport and PTP transactions.

use std::collections::VecDeque;
use std::process::Command;
use std::time::Duration;

use rusb::{DeviceHandle, Direction, GlobalContext, TransferType};

use crate::codec::{CallResult, Event};
use crate::error::Error;
use crate::ConnectOptions;

const MAX_CONTAINER: usize = 128 * 1024 * 1024;
const RESPONSE_OK: u16 = 0x2001;
const DEVICE_BUSY: u16 = 0x2019;

pub(crate) struct Session {
    handle: DeviceHandle<GlobalContext>,
    interface: u8,
    endpoint_out: u8,
    endpoint_in: u8,
    endpoint_interrupt: Option<u8>,
    timeout: Duration,
    transaction_id: u32,
    pending: Vec<u8>,
    events: VecDeque<Event>,
    closed: bool,
}

impl Session {
    pub(crate) fn try_new(options: &ConnectOptions) -> Result<Self, Error> {
        if options.release_system_client {
            release_system_client();
        }
        let mut session = Self::claim(options.timeout)?;
        if options.reset_device {
            let _ = session.handle.reset();
            std::thread::sleep(Duration::from_millis(800));
            if options.release_system_client {
                release_system_client();
            }
            match session.handle.claim_interface(session.interface) {
                Ok(()) | Err(rusb::Error::Busy) => {}
                Err(_) => {
                    drop(session);
                    if options.release_system_client {
                        release_system_client();
                    }
                    session = Self::claim(options.timeout)?;
                }
            }
        }
        Ok(session)
    }

    pub(crate) fn timeout(&self) -> Duration {
        self.timeout
    }

    pub(crate) fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    pub(crate) fn command(
        &mut self,
        code: u16,
        params: &[u32],
        data: Option<&[u8]>,
    ) -> Result<CallResult, Error> {
        if self.closed {
            return Err(Error::SessionClosed);
        }
        let mut attempt = 0;
        loop {
            let result = self.transact(code, params, data)?;
            if result.code == DEVICE_BUSY && attempt < 15 {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(80));
                continue;
            }
            self.drain_events();
            return Ok(result);
        }
    }

    pub(crate) fn command_ok(
        &mut self,
        code: u16,
        params: &[u32],
        data: Option<&[u8]>,
    ) -> Result<CallResult, Error> {
        let result = self.command(code, params, data)?;
        if result.code != RESPONSE_OK {
            return Err(Error::rejected(code, result.code));
        }
        Ok(result)
    }

    pub(crate) fn close_ptp_session(&mut self) -> Result<(), Error> {
        match self.transact(0x1003, &[], None) {
            Ok(result) if result.code == RESPONSE_OK || result.code == 0x2003 => Ok(()),
            Ok(result) => Err(Error::rejected(0x1003, result.code)),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn close_session(&mut self) -> Result<(), Error> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        self.close_ptp_session()
    }

    pub(crate) fn poll_event(&mut self, timeout: Duration) -> Result<Option<Event>, Error> {
        if let Some(event) = self.events.pop_front() {
            return Ok(Some(event));
        }
        let Some(endpoint) = self.endpoint_interrupt else {
            return Ok(None);
        };
        let mut buffer = [0u8; 256];
        match self.handle.read_interrupt(endpoint, &mut buffer, timeout) {
            Ok(size) => Ok(parse_event(&buffer[..size])),
            Err(rusb::Error::Timeout) => Ok(None),
            Err(rusb::Error::Pipe) => {
                let _ = self.handle.clear_halt(endpoint);
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn claim(timeout: Duration) -> Result<Self, Error> {
        for _ in 0..40 {
            if let Some(session) = try_claim(timeout)? {
                return Ok(session);
            }
            release_system_client();
            std::thread::sleep(Duration::from_millis(100));
        }
        Err(Error::CameraNotFound)
    }

    fn transact(
        &mut self,
        code: u16,
        params: &[u32],
        data: Option<&[u8]>,
    ) -> Result<CallResult, Error> {
        let transaction_id = self.allocate_transaction();
        let mut parameters = Vec::with_capacity(params.len() * 4);
        for param in params {
            parameters.extend_from_slice(&param.to_le_bytes());
        }
        self.write_container(1, code, transaction_id, &parameters)?;
        if let Some(payload) = data {
            self.write_container(2, code, transaction_id, payload)?;
        }

        let mut payload = Vec::new();
        for _ in 0..8 {
            let packet = self.read_container()?;
            if packet.len() < 12 {
                return Err(Error::Truncated);
            }
            let kind = le_u16(&packet[4..6])?;
            let packet_code = le_u16(&packet[6..8])?;
            match kind {
                2 => payload.extend_from_slice(&packet[12..]),
                3 => {
                    return Ok(CallResult {
                        code: packet_code,
                        params: u32_params(&packet[12..]),
                        data: payload,
                    });
                }
                4 => {
                    if let Some(event) = parse_event(&packet) {
                        self.events.push_back(event);
                    }
                }
                _ => return Err(Error::ContainerType { kind }),
            }
        }
        Err(Error::MissingResponse)
    }

    fn allocate_transaction(&mut self) -> u32 {
        let transaction_id = self.transaction_id;
        self.transaction_id = self.transaction_id.wrapping_add(1);
        if self.transaction_id == u32::MAX {
            self.transaction_id = 0;
        }
        transaction_id
    }

    fn write_container(
        &mut self,
        kind: u16,
        code: u16,
        transaction_id: u32,
        payload: &[u8],
    ) -> Result<(), Error> {
        let length = 12 + payload.len();
        if length > MAX_CONTAINER {
            return Err(Error::PayloadTooLarge);
        }
        let mut packet = Vec::with_capacity(length);
        packet.extend_from_slice(&(length as u32).to_le_bytes());
        packet.extend_from_slice(&kind.to_le_bytes());
        packet.extend_from_slice(&code.to_le_bytes());
        packet.extend_from_slice(&transaction_id.to_le_bytes());
        packet.extend_from_slice(payload);
        let mut offset = 0;
        while offset < packet.len() {
            match self
                .handle
                .write_bulk(self.endpoint_out, &packet[offset..], self.timeout)
            {
                Ok(0) => return Err(Error::Truncated),
                Ok(written) => offset += written,
                Err(rusb::Error::Pipe) => {
                    let _ = self.handle.clear_halt(self.endpoint_out);
                    return Err(Error::Stalled);
                }
                Err(rusb::Error::Timeout) => return Err(Error::Timeout),
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    fn read_container(&mut self) -> Result<Vec<u8>, Error> {
        loop {
            if self.pending.len() >= 4 {
                let length_bytes: [u8; 4] =
                    self.pending[..4].try_into().map_err(|_| Error::Truncated)?;
                let length = u32::from_le_bytes(length_bytes) as usize;
                if !(12..=MAX_CONTAINER).contains(&length) {
                    return Err(Error::ContainerLength { len: length });
                }
                if self.pending.len() >= length {
                    let packet = self.pending[..length].to_vec();
                    self.pending.drain(..length);
                    return Ok(packet);
                }
            }
            let mut buffer = vec![0u8; 64 * 1024];
            match self
                .handle
                .read_bulk(self.endpoint_in, &mut buffer, self.timeout)
            {
                Ok(0) => return Err(Error::Truncated),
                Ok(size) => self.pending.extend_from_slice(&buffer[..size]),
                Err(rusb::Error::Pipe) => {
                    let _ = self.handle.clear_halt(self.endpoint_in);
                    return Err(Error::Stalled);
                }
                Err(rusb::Error::Timeout) => return Err(Error::Timeout),
                Err(error) => return Err(error.into()),
            }
        }
    }

    fn drain_events(&mut self) {
        let Some(endpoint) = self.endpoint_interrupt else {
            return;
        };
        for _ in 0..16 {
            let mut buffer = [0u8; 256];
            match self
                .handle
                .read_interrupt(endpoint, &mut buffer, Duration::from_millis(1))
            {
                Ok(size) => {
                    if let Some(event) = parse_event(&buffer[..size]) {
                        self.events.push_back(event);
                    }
                }
                Err(rusb::Error::Pipe) => {
                    let _ = self.handle.clear_halt(endpoint);
                    break;
                }
                _ => break,
            }
        }
        while self.events.len() > 64 {
            self.events.pop_front();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.handle.release_interface(self.interface);
    }
}

fn try_claim(timeout: Duration) -> Result<Option<Session>, Error> {
    for device in rusb::devices()?.iter() {
        let Ok(descriptor) = device.device_descriptor() else {
            continue;
        };
        if descriptor.vendor_id() != 0x054c {
            continue;
        }
        let Ok(config) = device
            .active_config_descriptor()
            .or_else(|_| device.config_descriptor(0))
        else {
            continue;
        };
        let Some((interface, endpoint_out, endpoint_in, endpoint_interrupt)) =
            ptp_endpoints(&config)
        else {
            continue;
        };
        let Ok(handle) = device.open() else {
            continue;
        };
        let _ = handle.set_auto_detach_kernel_driver(true);
        if handle.claim_interface(interface).is_err() {
            continue;
        }
        return Ok(Some(Session {
            handle,
            interface,
            endpoint_out,
            endpoint_in,
            endpoint_interrupt,
            timeout,
            transaction_id: 0,
            pending: Vec::new(),
            events: VecDeque::new(),
            closed: false,
        }));
    }
    Ok(None)
}

fn ptp_endpoints(config: &rusb::ConfigDescriptor) -> Option<(u8, u8, u8, Option<u8>)> {
    for interface in config.interfaces() {
        for descriptor in interface.descriptors() {
            if descriptor.class_code() != 6 {
                continue;
            }
            let mut endpoint_out = None;
            let mut endpoint_in = None;
            let mut endpoint_interrupt = None;
            for endpoint in descriptor.endpoint_descriptors() {
                match (endpoint.transfer_type(), endpoint.direction()) {
                    (TransferType::Bulk, Direction::Out) => endpoint_out = Some(endpoint.address()),
                    (TransferType::Bulk, Direction::In) => endpoint_in = Some(endpoint.address()),
                    (TransferType::Interrupt, Direction::In) => {
                        endpoint_interrupt = Some(endpoint.address())
                    }
                    _ => {}
                }
            }
            if let (Some(endpoint_out), Some(endpoint_in)) = (endpoint_out, endpoint_in) {
                return Some((
                    descriptor.interface_number(),
                    endpoint_out,
                    endpoint_in,
                    endpoint_interrupt,
                ));
            }
        }
    }
    None
}

fn le_u16(bytes: &[u8]) -> Result<u16, Error> {
    let array: [u8; 2] = bytes.try_into().map_err(|_| Error::Truncated)?;
    Ok(u16::from_le_bytes(array))
}

fn u32_params(data: &[u8]) -> Vec<u32> {
    data.chunks_exact(4)
        .map(|chunk| {
            u32::from_le_bytes(
                <[u8; 4]>::try_from(chunk).expect("chunks_exact yields 4-byte groups"),
            )
        })
        .collect()
}

fn parse_event(packet: &[u8]) -> Option<Event> {
    if packet.len() < 12 {
        return None;
    }
    let length = u32::from_le_bytes(packet[0..4].try_into().ok()?) as usize;
    if length < 12 || length > packet.len() {
        return None;
    }
    Some(Event {
        code: u16::from_le_bytes(packet[6..8].try_into().ok()?),
        params: u32_params(&packet[12..length]),
    })
}

fn release_system_client() {
    if cfg!(target_os = "macos") {
        let _ = Command::new("killall")
            .args(["-9", "ptpcamerad"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}
