use std::ffi::CString;
use std::time::Duration;

use hidapi::{HidApi, HidDevice, HidError};
use thiserror::Error;

mod esphome;
mod rc;

pub use esphome::{
    ActionAcknowledgement, ActionSchemaMismatch, EspHomeError, EspHomeRcClient,
    MalformedMessageReason, ServerIdentity,
};
pub use rc::{
    MappingError, RC_CHANNEL_COUNT, RC_MAX_US, RC_MID_US, RC_MIN_US, RcChannels, RcMapping,
};

pub const VENDOR_ID: u16 = 0x1781;
pub const PRODUCT_ID: u16 = 0x0898;
pub const CHANNEL_COUNT: usize = 7;
pub const REPORT_SIZE: usize = 8;

const CHANNEL_BYTE_INDICES: [usize; CHANNEL_COUNT] = [0, 2, 3, 4, 5, 6, 7];
const MAX_REPORT_SIZE: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceIdentity {
    pub manufacturer: String,
    pub product: String,
    pub serial_number: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceState {
    channels: [u8; CHANNEL_COUNT],
    digital_switch: bool,
    raw: [u8; REPORT_SIZE],
}

impl DeviceState {
    #[must_use]
    pub const fn channels(&self) -> &[u8; CHANNEL_COUNT] {
        &self.channels
    }

    #[must_use]
    pub const fn digital_switch(&self) -> bool {
        self.digital_switch
    }

    #[must_use]
    pub const fn raw(&self) -> &[u8; REPORT_SIZE] {
        &self.raw
    }
}

impl TryFrom<&[u8]> for DeviceState {
    type Error = PacketError;

    fn try_from(report: &[u8]) -> Result<Self, Self::Error> {
        let raw: [u8; REPORT_SIZE] =
            report
                .try_into()
                .map_err(|_| PacketError::UnexpectedLength {
                    expected: REPORT_SIZE,
                    actual: report.len(),
                })?;
        let channels = CHANNEL_BYTE_INDICES.map(|index| raw[index]);

        Ok(Self {
            channels,
            digital_switch: raw[1] != 0,
            raw,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadStatus {
    Timeout,
    State(DeviceState),
}

pub struct SailiDevice {
    _api: HidApi,
    device: HidDevice,
    identity: DeviceIdentity,
}

impl SailiDevice {
    pub fn connect() -> Result<Self, SailiError> {
        let api = HidApi::new().map_err(SailiError::Initialize)?;
        let (path, identity) = find_adapter(&api)?;
        let device = api.open_path(path.as_c_str()).map_err(SailiError::Open)?;

        Ok(Self {
            _api: api,
            device,
            identity,
        })
    }

    #[must_use]
    pub const fn identity(&self) -> &DeviceIdentity {
        &self.identity
    }

    pub fn read_state(&self, timeout: Duration) -> Result<ReadStatus, SailiError> {
        let timeout_ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        let mut report = [0_u8; MAX_REPORT_SIZE];
        let count = self
            .device
            .read_timeout(&mut report, timeout_ms)
            .map_err(SailiError::Read)?;

        if count == 0 {
            return Ok(ReadStatus::Timeout);
        }

        let state = DeviceState::try_from(&report[..count])?;
        Ok(ReadStatus::State(state))
    }
}

fn find_adapter(api: &HidApi) -> Result<(CString, DeviceIdentity), SailiError> {
    api.device_list()
        .find(|info| info.vendor_id() == VENDOR_ID && info.product_id() == PRODUCT_ID)
        .map(|info| {
            (
                info.path().to_owned(),
                DeviceIdentity {
                    manufacturer: info.manufacturer_string().unwrap_or("Unknown").to_owned(),
                    product: info.product_string().unwrap_or("Unknown").to_owned(),
                    serial_number: info.serial_number().map(str::to_owned),
                },
            )
        })
        .ok_or(SailiError::AdapterNotFound)
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum PacketError {
    #[error("expected an {expected}-byte report, received {actual} bytes")]
    UnexpectedLength { expected: usize, actual: usize },
}

#[derive(Debug, Error)]
pub enum SailiError {
    #[error("could not initialize HID support")]
    Initialize(#[source] HidError),

    #[error("SAILI/PhoenixRC adapter {VENDOR_ID:04x}:{PRODUCT_ID:04x} was not found")]
    AdapterNotFound,

    #[error("could not open SAILI/PhoenixRC adapter")]
    Open(#[source] HidError),

    #[error("could not read SAILI/PhoenixRC adapter")]
    Read(#[source] HidError),

    #[error(transparent)]
    Packet(#[from] PacketError),
}
