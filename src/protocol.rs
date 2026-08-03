use std::fmt;
use std::str::FromStr;
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::{CHANNEL_COUNT, REPORT_SIZE};

const ANALOGUE_BYTE_INDICES: [usize; 6] = [0, 2, 3, 4, 5, 6];
const CALIBRATION_MIN_REPORTS: usize = 8;
const CALIBRATION_MAX_REPORTS: usize = 64;
const CALIBRATION_MOVEMENT_DELTA: u8 = 16;
const CADENCE_HISTORY: usize = 16;

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum PacketError {
    #[error("expected an {expected}-byte report, received {actual} bytes")]
    UnexpectedLength { expected: usize, actual: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReportFormat {
    Auto,
    RawMuxed8,
    LinuxDemuxed8,
    Legacy7Button,
}

impl ReportFormat {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::RawMuxed8 => "raw-muxed8",
            Self::LinuxDemuxed8 => "linux-demuxed8",
            Self::Legacy7Button => "legacy7-button",
        }
    }
}

impl fmt::Display for ReportFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

impl FromStr for ReportFormat {
    type Err = ReportFormatParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "auto" => Ok(Self::Auto),
            "raw-muxed8" => Ok(Self::RawMuxed8),
            "linux-demuxed8" => Ok(Self::LinuxDemuxed8),
            "legacy7-button" => Ok(Self::Legacy7Button),
            _ => Err(ReportFormatParseError {
                value: value.to_owned(),
            }),
        }
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
#[error("unknown report format {value:?}; use auto, raw-muxed8, linux-demuxed8, or legacy7-button")]
pub struct ReportFormatParseError {
    value: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RawReport {
    bytes: [u8; REPORT_SIZE],
    sequence: u64,
    received_at: Instant,
    interval: Option<Duration>,
    changed_mask: u8,
}

impl RawReport {
    pub fn try_new(
        bytes: &[u8],
        sequence: u64,
        received_at: Instant,
        previous: Option<&Self>,
    ) -> Result<Self, PacketError> {
        let bytes: [u8; REPORT_SIZE] =
            bytes
                .try_into()
                .map_err(|_| PacketError::UnexpectedLength {
                    expected: REPORT_SIZE,
                    actual: bytes.len(),
                })?;
        let changed_mask = previous.map_or(u8::MAX, |previous| {
            bytes
                .iter()
                .zip(previous.bytes)
                .enumerate()
                .fold(0, |mask, (index, (current, previous))| {
                    mask | u8::from(*current != previous) << index
                })
        });

        Ok(Self {
            bytes,
            sequence,
            received_at,
            interval: previous
                .map(|previous| received_at.saturating_duration_since(previous.received_at)),
            changed_mask,
        })
    }

    #[must_use]
    pub const fn bytes(&self) -> &[u8; REPORT_SIZE] {
        &self.bytes
    }

    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub const fn received_at(&self) -> Instant {
        self.received_at
    }

    #[must_use]
    pub const fn interval(&self) -> Option<Duration> {
        self.interval
    }

    #[must_use]
    pub const fn changed_mask(&self) -> u8 {
        self.changed_mask
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodedState {
    channels: [Option<u8>; CHANNEL_COUNT],
    legacy_button: Option<bool>,
    raw: RawReport,
    format: ReportFormat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MuxState {
    Uncalibrated,
    CalibratingFirst,
    CalibratingSecond,
    Calibrated,
    Lost,
}

impl MuxState {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Uncalibrated => "uncalibrated",
            Self::CalibratingFirst => "calibrating input 7",
            Self::CalibratingSecond => "calibrating input 8",
            Self::Calibrated => "calibrated",
            Self::Lost => "lost",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MuxCalibrationError {
    #[error("mux calibration is only available for raw-muxed8 reports")]
    UnsupportedFormat,
    #[error("mux calibration is already in progress")]
    AlreadyInProgress,
    #[error("mux calibration is not in progress")]
    NotInProgress,
    #[error("move the requested control through a larger range before confirming calibration")]
    InsufficientMovement,
    #[error("both mux phases changed; move only the requested control during calibration")]
    BothPhasesMoved,
    #[error("the second calibration control did not use the opposite mux phase")]
    WrongPhaseMoved,
    #[error("mux calibration window expired; restart calibration")]
    WindowExpired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MuxLossReason {
    #[error("report cadence gap detected")]
    CadenceGap,
    #[error("malformed report")]
    MalformedReport,
    #[error("HID read failed")]
    ReadError,
    #[error("device reconnected")]
    Reconnected,
}

impl DecodedState {
    #[must_use]
    pub const fn channels(&self) -> &[Option<u8>; CHANNEL_COUNT] {
        &self.channels
    }

    #[must_use]
    pub const fn channel(&self, index: usize) -> Option<u8> {
        if index < CHANNEL_COUNT {
            self.channels[index]
        } else {
            None
        }
    }

    #[must_use]
    pub const fn legacy_button(&self) -> Option<bool> {
        self.legacy_button
    }

    #[must_use]
    pub const fn raw(&self) -> &RawReport {
        &self.raw
    }

    #[must_use]
    pub const fn format(&self) -> ReportFormat {
        self.format
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecoderStatus {
    pub selected_format: Option<ReportFormat>,
    pub mux_state: Option<MuxState>,
    pub mux_seen: [bool; 2],
    pub current_phase: Option<usize>,
    pub calibration_samples: usize,
    pub calibration_target: usize,
    pub calibration_error: Option<MuxCalibrationError>,
    pub loss_reason: Option<MuxLossReason>,
}

pub struct Decoder {
    format: ReportFormat,
    mux: MuxPhaseTracker,
}

impl Decoder {
    #[must_use]
    pub const fn new(format: ReportFormat, swap_mux_channels: bool) -> Self {
        Self {
            format,
            mux: MuxPhaseTracker::new(swap_mux_channels),
        }
    }

    pub fn decode(&mut self, raw: RawReport) -> Result<Option<DecodedState>, PacketError> {
        match self.format {
            ReportFormat::Auto => Ok(None),
            ReportFormat::RawMuxed8 => Ok(self.mux.decode(raw)),
            ReportFormat::LinuxDemuxed8 => Ok(Some(self.decode_linux_demuxed(raw))),
            ReportFormat::Legacy7Button => Ok(Some(self.decode_legacy(raw))),
        }
    }

    pub fn start_mux_calibration(&mut self) -> Result<(), MuxCalibrationError> {
        if self.format != ReportFormat::RawMuxed8 {
            return Err(MuxCalibrationError::UnsupportedFormat);
        }
        self.mux.start_calibration()
    }

    pub fn confirm_mux_calibration(&mut self) -> Result<(), MuxCalibrationError> {
        if self.format != ReportFormat::RawMuxed8 {
            return Err(MuxCalibrationError::UnsupportedFormat);
        }
        self.mux.confirm_calibration()
    }

    pub fn mark_lost(&mut self, reason: MuxLossReason) {
        if self.format == ReportFormat::RawMuxed8 {
            self.mux.mark_lost(reason);
        }
    }

    #[must_use]
    pub const fn status(&self) -> DecoderStatus {
        DecoderStatus {
            selected_format: match self.format {
                ReportFormat::Auto => None,
                format => Some(format),
            },
            mux_state: match self.format {
                ReportFormat::RawMuxed8 => Some(self.mux.state),
                ReportFormat::Auto | ReportFormat::LinuxDemuxed8 | ReportFormat::Legacy7Button => {
                    None
                }
            },
            mux_seen: self.mux.seen(),
            current_phase: self.mux.last_phase,
            calibration_samples: self.mux.calibration_samples(),
            calibration_target: CALIBRATION_MIN_REPORTS,
            calibration_error: self.mux.calibration_error,
            loss_reason: self.mux.loss_reason,
        }
    }

    pub fn reset(&mut self) {
        self.mux.reset();
    }

    fn decode_linux_demuxed(&self, raw: RawReport) -> DecodedState {
        let mut channels = [None; CHANNEL_COUNT];
        for (channel, byte) in ANALOGUE_BYTE_INDICES.into_iter().enumerate() {
            channels[channel] = Some(raw.bytes()[byte]);
        }
        channels[6] = Some(raw.bytes()[1]);
        channels[7] = Some(raw.bytes()[7]);
        DecodedState {
            channels,
            legacy_button: None,
            raw,
            format: ReportFormat::LinuxDemuxed8,
        }
    }

    fn decode_legacy(&self, raw: RawReport) -> DecodedState {
        let mut channels = [None; CHANNEL_COUNT];
        for (channel, byte) in [0, 2, 3, 4, 5, 6, 7].into_iter().enumerate() {
            channels[channel] = Some(raw.bytes()[byte]);
        }
        DecodedState {
            channels,
            legacy_button: Some(raw.bytes()[1] != 0),
            raw,
            format: ReportFormat::Legacy7Button,
        }
    }
}

struct MuxPhaseTracker {
    swap_mux_channels: bool,
    state: MuxState,
    mux_values: [Option<u8>; 2],
    input_seven_phase: Option<usize>,
    next_phase: usize,
    last_phase: Option<usize>,
    calibration: CalibrationObservation,
    calibration_error: Option<MuxCalibrationError>,
    loss_reason: Option<MuxLossReason>,
    cadence: CadenceTracker,
}

impl MuxPhaseTracker {
    const fn new(swap_mux_channels: bool) -> Self {
        Self {
            swap_mux_channels,
            state: MuxState::Uncalibrated,
            mux_values: [None; 2],
            input_seven_phase: None,
            next_phase: 0,
            last_phase: None,
            calibration: CalibrationObservation::new(),
            calibration_error: None,
            loss_reason: None,
            cadence: CadenceTracker::new(),
        }
    }

    fn decode(&mut self, raw: RawReport) -> Option<DecodedState> {
        if self.state == MuxState::Lost {
            return None;
        }
        if self.cadence.observe(raw.interval()) {
            self.mark_lost(MuxLossReason::CadenceGap);
            return None;
        }

        let phase = self.next_phase;
        self.next_phase ^= 1;
        self.last_phase = Some(phase);

        if matches!(
            self.state,
            MuxState::CalibratingFirst | MuxState::CalibratingSecond
        ) {
            self.calibration.observe(phase, raw.bytes()[7]);
            if self.calibration.samples > CALIBRATION_MAX_REPORTS {
                self.state = MuxState::Uncalibrated;
                self.calibration_error = Some(MuxCalibrationError::WindowExpired);
                self.calibration.reset();
            }
            return None;
        }

        if self.state != MuxState::Calibrated {
            return None;
        }

        self.mux_values[phase] = Some(raw.bytes()[7]);
        let (Some(first), Some(second)) = (self.mux_values[0], self.mux_values[1]) else {
            return None;
        };

        let input_seven_phase = self.input_seven_phase?;
        let input_eight_phase = input_seven_phase ^ 1;
        let mut channels = [None; CHANNEL_COUNT];
        for (channel, byte) in ANALOGUE_BYTE_INDICES.into_iter().enumerate() {
            channels[channel] = Some(raw.bytes()[byte]);
        }
        let muxed = [first, second];
        if self.swap_mux_channels {
            channels[6] = Some(muxed[input_eight_phase]);
            channels[7] = Some(muxed[input_seven_phase]);
        } else {
            channels[6] = Some(muxed[input_seven_phase]);
            channels[7] = Some(muxed[input_eight_phase]);
        }

        Some(DecodedState {
            channels,
            legacy_button: None,
            raw,
            format: ReportFormat::RawMuxed8,
        })
    }

    fn start_calibration(&mut self) -> Result<(), MuxCalibrationError> {
        if matches!(
            self.state,
            MuxState::CalibratingFirst | MuxState::CalibratingSecond
        ) {
            return Err(MuxCalibrationError::AlreadyInProgress);
        }
        self.state = MuxState::CalibratingFirst;
        self.mux_values = [None; 2];
        self.input_seven_phase = None;
        self.next_phase = 0;
        self.last_phase = None;
        self.calibration.reset();
        self.calibration_error = None;
        self.loss_reason = None;
        self.cadence.reset();
        Ok(())
    }

    fn confirm_calibration(&mut self) -> Result<(), MuxCalibrationError> {
        let changed = self.calibration.changed_phases();
        if self.calibration.samples < CALIBRATION_MIN_REPORTS {
            return self.fail_calibration(MuxCalibrationError::InsufficientMovement);
        }
        if changed == [true, true] {
            return self.fail_calibration(MuxCalibrationError::BothPhasesMoved);
        }
        let Some(changed_phase) = changed
            .iter()
            .enumerate()
            .find_map(|(phase, changed)| (*changed).then_some(phase))
        else {
            return self.fail_calibration(MuxCalibrationError::InsufficientMovement);
        };

        match self.state {
            MuxState::CalibratingFirst => {
                self.input_seven_phase = Some(changed_phase);
                self.state = MuxState::CalibratingSecond;
                self.calibration.reset();
                self.calibration_error = None;
                Ok(())
            }
            MuxState::CalibratingSecond => {
                if self.input_seven_phase == Some(changed_phase) {
                    return self.fail_calibration(MuxCalibrationError::WrongPhaseMoved);
                }
                self.state = MuxState::Calibrated;
                self.calibration.reset();
                self.calibration_error = None;
                self.loss_reason = None;
                Ok(())
            }
            _ => Err(MuxCalibrationError::NotInProgress),
        }
    }

    fn fail_calibration(&mut self, error: MuxCalibrationError) -> Result<(), MuxCalibrationError> {
        self.state = MuxState::Uncalibrated;
        self.calibration_error = Some(error);
        self.calibration.reset();
        Err(error)
    }

    fn mark_lost(&mut self, reason: MuxLossReason) {
        self.state = MuxState::Lost;
        self.mux_values = [None; 2];
        self.input_seven_phase = None;
        self.last_phase = None;
        self.calibration.reset();
        self.calibration_error = None;
        self.loss_reason = Some(reason);
        self.cadence.reset();
    }

    fn reset(&mut self) {
        self.state = MuxState::Uncalibrated;
        self.mux_values = [None; 2];
        self.input_seven_phase = None;
        self.next_phase = 0;
        self.last_phase = None;
        self.calibration.reset();
        self.calibration_error = None;
        self.loss_reason = None;
        self.cadence.reset();
    }

    const fn seen(&self) -> [bool; 2] {
        [self.mux_values[0].is_some(), self.mux_values[1].is_some()]
    }

    const fn calibration_samples(&self) -> usize {
        self.calibration.samples
    }
}

#[derive(Clone, Copy)]
struct CalibrationObservation {
    minimum: [u8; 2],
    maximum: [u8; 2],
    samples: usize,
}

impl CalibrationObservation {
    const fn new() -> Self {
        Self {
            minimum: [u8::MAX; 2],
            maximum: [u8::MIN; 2],
            samples: 0,
        }
    }

    fn observe(&mut self, phase: usize, value: u8) {
        self.minimum[phase] = self.minimum[phase].min(value);
        self.maximum[phase] = self.maximum[phase].max(value);
        self.samples = self.samples.saturating_add(1);
    }

    fn changed_phases(&self) -> [bool; 2] {
        [
            self.maximum[0].saturating_sub(self.minimum[0]) >= CALIBRATION_MOVEMENT_DELTA,
            self.maximum[1].saturating_sub(self.minimum[1]) >= CALIBRATION_MOVEMENT_DELTA,
        ]
    }

    const fn reset(&mut self) {
        *self = Self::new();
    }
}

struct CadenceTracker {
    history: Vec<Duration>,
}

impl CadenceTracker {
    const fn new() -> Self {
        Self {
            history: Vec::new(),
        }
    }

    fn observe(&mut self, interval: Option<Duration>) -> bool {
        let Some(interval) = interval.filter(|interval| !interval.is_zero()) else {
            return false;
        };
        if let Some(median) = self.median()
            && interval >= median.saturating_mul(2)
        {
            return true;
        }
        self.history.push(interval);
        if self.history.len() > CADENCE_HISTORY {
            self.history.remove(0);
        }
        false
    }

    fn median(&self) -> Option<Duration> {
        if self.history.len() < 4 {
            return None;
        }
        let mut values = self.history.clone();
        values.sort_unstable();
        Some(values[values.len() / 2])
    }

    fn reset(&mut self) {
        self.history.clear();
    }
}
