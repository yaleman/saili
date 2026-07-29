use thiserror::Error;

use crate::{CHANNEL_COUNT, DeviceState};

pub const RC_CHANNEL_COUNT: usize = 16;
pub const RC_MIN_US: u16 = 988;
pub const RC_MID_US: u16 = 1500;
pub const RC_MAX_US: u16 = 2012;

const PRIMARY_CHANNEL_COUNT: usize = 4;

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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RcMapping {
    primary_indices: [usize; PRIMARY_CHANNEL_COUNT],
    inverted: [bool; PRIMARY_CHANNEL_COUNT],
}

impl RcMapping {
    pub fn new(
        one_based_channels: [usize; PRIMARY_CHANNEL_COUNT],
        inverted: [bool; PRIMARY_CHANNEL_COUNT],
    ) -> Result<Self, MappingError> {
        let mut seen = [false; CHANNEL_COUNT];
        let mut primary_indices = [0; PRIMARY_CHANNEL_COUNT];

        for (position, one_based) in one_based_channels.into_iter().enumerate() {
            if !(1..=CHANNEL_COUNT).contains(&one_based) {
                return Err(MappingError::ChannelOutOfRange { channel: one_based });
            }

            let index = one_based - 1;
            if seen[index] {
                return Err(MappingError::DuplicateChannel { channel: one_based });
            }
            seen[index] = true;
            primary_indices[position] = index;
        }

        Ok(Self {
            primary_indices,
            inverted,
        })
    }

    #[must_use]
    pub fn map(&self, state: DeviceState) -> RcChannels {
        let raw = state.channels();
        let mut values = *RcChannels::safe().values();

        for output in 0..PRIMARY_CHANNEL_COUNT {
            values[output] =
                scale_analogue(raw[self.primary_indices[output]], self.inverted[output]);
        }

        values[4] = if state.digital_switch() {
            RC_MAX_US
        } else {
            RC_MIN_US
        };

        let mut auxiliary_output = 5;
        for (index, value) in raw.iter().copied().enumerate() {
            if !self.primary_indices.contains(&index) && auxiliary_output < RC_CHANNEL_COUNT {
                values[auxiliary_output] = scale_analogue(value, false);
                auxiliary_output += 1;
            }
        }

        RcChannels { values }
    }
}

impl Default for RcMapping {
    fn default() -> Self {
        Self {
            primary_indices: [0, 1, 2, 3],
            inverted: [false; PRIMARY_CHANNEL_COUNT],
        }
    }
}

#[must_use]
fn scale_analogue(value: u8, inverted: bool) -> u16 {
    let value = if inverted { u8::MAX - value } else { value };

    if (126..=129).contains(&value) {
        return RC_MID_US;
    }

    RC_MIN_US + ((u32::from(value) * 1024 + 127) / 255) as u16
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum MappingError {
    #[error("channel {channel} is outside the valid 1-{CHANNEL_COUNT} range")]
    ChannelOutOfRange { channel: usize },

    #[error("channel {channel} is assigned to more than one primary control")]
    DuplicateChannel { channel: usize },
}
