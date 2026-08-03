use thiserror::Error;

use crate::{CHANNEL_COUNT, DecodedState};

pub const RC_CHANNEL_COUNT: usize = 16;
pub const RC_MIN_US: u16 = 988;
pub const RC_MID_US: u16 = 1500;
pub const RC_MAX_US: u16 = 2012;

const PRIMARY_CHANNEL_COUNT: usize = 4;
const AUXILIARY_CHANNEL_COUNT: usize = CHANNEL_COUNT - PRIMARY_CHANNEL_COUNT;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelCalibration {
    pub minimum: u8,
    pub centre: u8,
    pub maximum: u8,
    pub deadband: u8,
}

impl Default for ChannelCalibration {
    fn default() -> Self {
        Self {
            minimum: 0,
            centre: 127,
            maximum: u8::MAX,
            deadband: 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArmConfig {
    pub channel: Option<usize>,
    pub threshold: u8,
    pub hysteresis: u8,
    pub inverted: bool,
}

impl Default for ArmConfig {
    fn default() -> Self {
        Self {
            channel: None,
            threshold: 127,
            hysteresis: 4,
            inverted: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArmController {
    config: ArmConfig,
    armed: bool,
}

impl ArmController {
    #[must_use]
    pub const fn new(config: ArmConfig) -> Self {
        Self {
            config,
            armed: false,
        }
    }

    pub fn update(&mut self, state: DecodedState) -> bool {
        let Some(channel) = self.config.channel else {
            self.armed = false;
            return false;
        };
        let Some(mut value) = state.channel(channel) else {
            self.armed = false;
            return false;
        };
        if self.config.inverted {
            value = u8::MAX - value;
        }
        let threshold = self.config.threshold;
        let hysteresis = self.config.hysteresis;
        if self.armed {
            if value <= threshold.saturating_sub(hysteresis) {
                self.armed = false;
            }
        } else if value >= threshold.saturating_add(hysteresis) {
            self.armed = true;
        }
        self.armed
    }

    pub const fn reset(&mut self) {
        self.armed = false;
    }

    #[must_use]
    pub const fn is_armed(&self) -> bool {
        self.armed
    }

    #[must_use]
    pub const fn has_source(&self) -> bool {
        self.config.channel.is_some()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RcChannels {
    values: [u16; RC_CHANNEL_COUNT],
}

impl RcChannels {
    #[must_use]
    pub const fn safe() -> Self {
        Self {
            values: [
                RC_MID_US, RC_MID_US, RC_MIN_US, RC_MID_US, RC_MIN_US, RC_MIN_US, RC_MIN_US,
                RC_MIN_US, RC_MIN_US, RC_MIN_US, RC_MIN_US, RC_MIN_US, RC_MIN_US, RC_MIN_US,
                RC_MIN_US, RC_MIN_US,
            ],
        }
    }

    #[must_use]
    pub const fn values(&self) -> &[u16; RC_CHANNEL_COUNT] {
        &self.values
    }

    #[must_use]
    pub const fn roll(&self) -> u16 {
        self.values[0]
    }

    #[must_use]
    pub const fn pitch(&self) -> u16 {
        self.values[1]
    }

    #[must_use]
    pub const fn throttle(&self) -> u16 {
        self.values[2]
    }

    #[must_use]
    pub const fn yaw(&self) -> u16 {
        self.values[3]
    }

    #[must_use]
    pub const fn armed(&self) -> bool {
        self.values[4] > RC_MID_US
    }

    #[must_use]
    pub const fn with_arm(self, armed: bool) -> Self {
        let mut values = self.values;
        values[4] = if armed { RC_MAX_US } else { RC_MIN_US };
        Self { values }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RcMapping {
    primary_indices: [usize; PRIMARY_CHANNEL_COUNT],
    auxiliary_indices: [usize; AUXILIARY_CHANNEL_COUNT],
    inverted: [bool; CHANNEL_COUNT],
    calibration: [ChannelCalibration; CHANNEL_COUNT],
}

impl RcMapping {
    pub fn new(
        one_based_channels: [usize; PRIMARY_CHANNEL_COUNT],
        inverted: [bool; PRIMARY_CHANNEL_COUNT],
    ) -> Result<Self, MappingError> {
        let mut all_channels = [0; CHANNEL_COUNT];
        all_channels[..PRIMARY_CHANNEL_COUNT].copy_from_slice(&one_based_channels);
        let mut next = PRIMARY_CHANNEL_COUNT;
        for channel in 1..=CHANNEL_COUNT {
            if !one_based_channels.contains(&channel) {
                all_channels[next] = channel;
                next += 1;
            }
        }
        let mut all_inverted = [false; CHANNEL_COUNT];
        all_inverted[..PRIMARY_CHANNEL_COUNT].copy_from_slice(&inverted);
        Self::new_full(all_channels, all_inverted)
    }

    pub fn new_full(
        one_based_channels: [usize; CHANNEL_COUNT],
        inverted: [bool; CHANNEL_COUNT],
    ) -> Result<Self, MappingError> {
        let mut seen = [false; CHANNEL_COUNT];
        for &channel in &one_based_channels {
            if !(1..=CHANNEL_COUNT).contains(&channel) {
                return Err(MappingError::ChannelOutOfRange { channel });
            }
            let index = channel - 1;
            if seen[index] {
                return Err(MappingError::DuplicateChannel { channel });
            }
            seen[index] = true;
        }

        let mut primary_indices = [0; PRIMARY_CHANNEL_COUNT];
        for (index, channel) in one_based_channels[..PRIMARY_CHANNEL_COUNT]
            .iter()
            .enumerate()
        {
            primary_indices[index] = channel - 1;
        }
        let mut auxiliary_indices = [0; AUXILIARY_CHANNEL_COUNT];
        for (index, channel) in one_based_channels[PRIMARY_CHANNEL_COUNT..]
            .iter()
            .enumerate()
        {
            auxiliary_indices[index] = channel - 1;
        }

        Ok(Self {
            primary_indices,
            auxiliary_indices,
            inverted,
            calibration: [ChannelCalibration::default(); CHANNEL_COUNT],
        })
    }

    #[must_use]
    pub const fn with_calibration(
        mut self,
        calibration: [ChannelCalibration; CHANNEL_COUNT],
    ) -> Self {
        self.calibration = calibration;
        self
    }

    #[must_use]
    pub const fn primary_channels(&self) -> [usize; PRIMARY_CHANNEL_COUNT] {
        [
            self.primary_indices[0] + 1,
            self.primary_indices[1] + 1,
            self.primary_indices[2] + 1,
            self.primary_indices[3] + 1,
        ]
    }

    #[must_use]
    pub const fn all_channels(&self) -> [usize; CHANNEL_COUNT] {
        [
            self.primary_indices[0] + 1,
            self.primary_indices[1] + 1,
            self.primary_indices[2] + 1,
            self.primary_indices[3] + 1,
            self.auxiliary_indices[0] + 1,
            self.auxiliary_indices[1] + 1,
            self.auxiliary_indices[2] + 1,
            self.auxiliary_indices[3] + 1,
        ]
    }

    #[must_use]
    pub const fn inverted(&self) -> [bool; CHANNEL_COUNT] {
        self.inverted
    }

    #[must_use]
    pub const fn calibration(&self) -> [ChannelCalibration; CHANNEL_COUNT] {
        self.calibration
    }

    #[must_use]
    pub fn map(&self, state: DecodedState) -> RcChannels {
        let mut values = *RcChannels::safe().values();

        for (output, value) in values.iter_mut().enumerate().take(PRIMARY_CHANNEL_COUNT) {
            *value = state.channel(self.primary_indices[output]).map_or_else(
                || safe_primary_value(output),
                |value| self.scale(value, output),
            );
        }

        for (offset, &input) in self.auxiliary_indices.iter().enumerate() {
            values[5 + offset] = state.channel(input).map_or(RC_MIN_US, |value| {
                self.scale(value, PRIMARY_CHANNEL_COUNT + offset)
            });
        }

        RcChannels { values }
    }

    fn scale(&self, value: u8, output: usize) -> u16 {
        scale_analogue(value, self.inverted[output], self.calibration[output])
    }
}

impl Default for RcMapping {
    fn default() -> Self {
        Self {
            primary_indices: [0, 1, 2, 3],
            auxiliary_indices: [4, 5, 6, 7],
            inverted: [false; CHANNEL_COUNT],
            calibration: [ChannelCalibration::default(); CHANNEL_COUNT],
        }
    }
}

fn safe_primary_value(output: usize) -> u16 {
    if output == 2 { RC_MIN_US } else { RC_MID_US }
}

#[must_use]
fn scale_analogue(value: u8, inverted: bool, calibration: ChannelCalibration) -> u16 {
    let value = if inverted { u8::MAX - value } else { value };
    let centre = calibration.centre;
    let distance = value.abs_diff(centre);
    if distance <= calibration.deadband {
        return RC_MID_US;
    }

    if value < centre {
        let span = u32::from(centre.saturating_sub(calibration.minimum).max(1));
        let offset = u32::from(value.saturating_sub(calibration.minimum));
        RC_MIN_US + ((offset * u32::from(RC_MID_US - RC_MIN_US) + span / 2) / span) as u16
    } else {
        let span = u32::from(calibration.maximum.saturating_sub(centre).max(1));
        let offset = u32::from(value.saturating_sub(centre));
        RC_MID_US + ((offset * u32::from(RC_MAX_US - RC_MID_US) + span / 2) / span) as u16
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum MappingError {
    #[error("channel {channel} is outside the valid 1-{CHANNEL_COUNT} range")]
    ChannelOutOfRange { channel: usize },

    #[error("channel {channel} is assigned to more than one output")]
    DuplicateChannel { channel: usize },
}
