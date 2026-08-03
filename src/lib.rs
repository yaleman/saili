use std::ffi::CString;
use std::time::{Duration, Instant};

use hidapi::{HidApi, HidDevice, HidError};
use thiserror::Error;

mod crsf;
mod esphome;
mod protocol;
mod rc;
mod reader;

pub use crsf::{
    CRSF_FRAME_SIZE_MAX, CRSF_FRAME_TYPE_ATTITUDE, CRSF_FRAME_TYPE_BAROMETER,
    CRSF_FRAME_TYPE_BAROMETRIC_ALTITUDE, CRSF_FRAME_TYPE_BATTERY, CRSF_FRAME_TYPE_DEVICE_INFO,
    CRSF_FRAME_TYPE_FLIGHT_MODE, CRSF_FRAME_TYPE_GPS, CRSF_FRAME_TYPE_HEARTBEAT,
    CRSF_FRAME_TYPE_MAGNETOMETER, CRSF_FRAME_TYPE_MSP_RESPONSE, CRSF_FRAME_TYPE_RANGE,
    CRSF_FRAME_TYPE_VARIO, CrsfError, CrsfFrame, CrsfTelemetry, crc8_dvb_s2,
};
pub use esphome::{
    ActionAcknowledgement, ActionSchemaMismatch, CommandExchange, EspHomeError, EspHomeRcClient,
    MalformedMessageReason, ServerIdentity,
};
pub use protocol::{
    DecodedState, Decoder, DecoderStatus, MuxCalibrationError, MuxLossReason, MuxState,
    PacketError, RawReport, ReportFormat, ReportFormatParseError,
};
pub use rc::{
    ArmConfig, ArmController, ChannelCalibration, MappingError, RC_CHANNEL_COUNT, RC_MAX_US,
    RC_MID_US, RC_MIN_US, RcChannels, RcMapping,
};
pub use reader::{
    FormatConfidence, ReaderCommand, ReaderCommandError, ReaderConfig, ReaderHandle,
    ReaderSnapshot, ReaderStartError, ReaderStats,
};

pub const VENDOR_ID: u16 = 0x1781;
pub const PRODUCT_ID: u16 = 0x0898;
pub const CHANNEL_COUNT: usize = 8;
pub const REPORT_SIZE: usize = 8;
pub const MAX_REPORT_SIZE: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceIdentity {
    pub manufacturer: String,
    pub product: String,
    pub serial_number: Option<String>,
    pub path: String,
    pub usage_page: u16,
    pub usage: u16,
    pub interface_number: i32,
    pub descriptor_hash: Option<String>,
    pub kernel_driver: Option<String>,
}

impl DeviceIdentity {
    #[must_use]
    pub fn format_hint(&self) -> Option<ReportFormat> {
        if self
            .kernel_driver
            .as_deref()
            .is_some_and(|driver| driver.eq_ignore_ascii_case("pxrc"))
        {
            return Some(ReportFormat::LinuxDemuxed8);
        }

        let identity = format!("{} {}", self.manufacturer, self.product).to_ascii_lowercase();
        ["feiying", "goldwarrior", "khobby"]
            .iter()
            .any(|name| identity.contains(name))
            .then_some(ReportFormat::RawMuxed8)
    }
}

#[derive(Clone, Debug)]
pub struct HidReport {
    bytes: Vec<u8>,
    received_at: Instant,
}

impl HidReport {
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub const fn received_at(&self) -> Instant {
        self.received_at
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadReportStatus {
    Timeout,
    Report {
        bytes: Vec<u8>,
        received_at: Instant,
    },
}

pub struct SailiDevice {
    pub(crate) api: HidApi,
    pub(crate) device: HidDevice,
    pub(crate) identity: DeviceIdentity,
    pub(crate) path: CString,
}

impl SailiDevice {
    pub fn connect() -> Result<Self, SailiError> {
        let api = HidApi::new().map_err(SailiError::Initialize)?;
        let (path, mut identity) = find_adapter(&api)?;
        let device = api.open_path(path.as_c_str()).map_err(SailiError::Open)?;
        identity.descriptor_hash = report_descriptor_hash(&device);

        Ok(Self {
            api,
            device,
            identity,
            path,
        })
    }

    #[must_use]
    pub const fn identity(&self) -> &DeviceIdentity {
        &self.identity
    }

    pub fn read_report(&self, timeout: Duration) -> Result<ReadReportStatus, SailiError> {
        let timeout_ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        let mut report = [0_u8; MAX_REPORT_SIZE];
        let count = self
            .device
            .read_timeout(&mut report, timeout_ms)
            .map_err(SailiError::Read)?;

        if count == 0 {
            return Ok(ReadReportStatus::Timeout);
        }

        Ok(ReadReportStatus::Report {
            bytes: report[..count].to_vec(),
            received_at: Instant::now(),
        })
    }

    pub(crate) fn reconnect(&mut self) -> Result<(), SailiError> {
        self.api.refresh_devices().map_err(SailiError::Initialize)?;
        let (path, mut identity) = find_adapter(&self.api)?;
        let device = self
            .api
            .open_path(path.as_c_str())
            .map_err(SailiError::Open)?;
        identity.descriptor_hash = report_descriptor_hash(&device);
        self.path = path;
        self.identity = identity;
        self.device = device;
        Ok(())
    }
}

fn find_adapter(api: &HidApi) -> Result<(CString, DeviceIdentity), SailiError> {
    api.device_list()
        .find(|info| info.vendor_id() == VENDOR_ID && info.product_id() == PRODUCT_ID)
        .map(|info| {
            let path = info.path().to_owned();
            (
                path.clone(),
                DeviceIdentity {
                    manufacturer: info.manufacturer_string().unwrap_or("Unknown").to_owned(),
                    product: info.product_string().unwrap_or("Unknown").to_owned(),
                    serial_number: info.serial_number().map(str::to_owned),
                    path: path.to_string_lossy().into_owned(),
                    usage_page: info.usage_page(),
                    usage: info.usage(),
                    interface_number: info.interface_number(),
                    descriptor_hash: None,
                    kernel_driver: kernel_driver_for_path(info.path()),
                },
            )
        })
        .ok_or(SailiError::AdapterNotFound)
}

fn report_descriptor_hash(device: &HidDevice) -> Option<String> {
    let mut descriptor = [0_u8; hidapi::MAX_REPORT_DESCRIPTOR_SIZE];
    let count = device.get_report_descriptor(&mut descriptor).ok()?;
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in &descriptor[..count] {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    Some(format!("{hash:016x}"))
}

#[cfg(target_os = "linux")]
fn kernel_driver_for_path(path: &std::ffi::CStr) -> Option<String> {
    let path = path.to_string_lossy();
    let name = path.rsplit('/').next()?;
    std::fs::read_link(format!("/sys/class/hidraw/{name}/device/driver"))
        .ok()?
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

#[cfg(not(target_os = "linux"))]
fn kernel_driver_for_path(_path: &std::ffi::CStr) -> Option<String> {
    None
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
