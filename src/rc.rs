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
    auxiliary_indices: [usize; 3],
    inverted: [bool; CHANNEL_COUNT],
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

        let mut auxiliary_indices = [0; 3];
        let mut auxiliary_output = 0;
        for input in 0..CHANNEL_COUNT {
            if !primary_indices.contains(&input) {
                auxiliary_indices[auxiliary_output] = input;
                auxiliary_output += 1;
            }
        }

        let mut all_inverted = [false; CHANNEL_COUNT];
        all_inverted[..PRIMARY_CHANNEL_COUNT].copy_from_slice(&inverted);
        Ok(Self {
            primary_indices,
            auxiliary_indices,
            inverted: all_inverted,
        })
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

        Ok(Self {
            primary_indices: [
                one_based_channels[0] - 1,
                one_based_channels[1] - 1,
                one_based_channels[2] - 1,
                one_based_channels[3] - 1,
            ],
            auxiliary_indices: [
                one_based_channels[4] - 1,
                one_based_channels[5] - 1,
                one_based_channels[6] - 1,
            ],
            inverted,
        })
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
        ]
    }

    #[must_use]
    pub const fn inverted(&self) -> [bool; CHANNEL_COUNT] {
        self.inverted
    }

    #[must_use]
    pub fn map(&self, state: DeviceState) -> RcChannels {
        let raw = state.channels();
        let mut values = *RcChannels::safe().values();

        for output in 0..PRIMARY_CHANNEL_COUNT {
            values[output] =
                scale_analogue(raw[self.primary_indices[output]], self.inverted[output]);
        }

        // The SAILI mode selectors are not reported as controller inputs.
        // Live forwarding is the arm request; SAFE HOLD overrides this with
        // the disarmed channel set before it reaches the bridge.
        values[4] = RC_MAX_US;

        for (offset, &input) in self.auxiliary_indices.iter().enumerate() {
            values[5 + offset] = scale_analogue(raw[input], self.inverted[4 + offset]);
        }

        RcChannels { values }
    }
}

impl Default for RcMapping {
    fn default() -> Self {
        Self {
            primary_indices: [0, 1, 2, 3],
            auxiliary_indices: [4, 5, 6],
            inverted: [false; CHANNEL_COUNT],
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
