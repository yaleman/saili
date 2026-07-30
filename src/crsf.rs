use thiserror::Error;

pub const CRSF_FRAME_SIZE_MAX: usize = 64;
pub const CRSF_FRAME_TYPE_GPS: u8 = 0x02;
pub const CRSF_FRAME_TYPE_VARIO: u8 = 0x07;
pub const CRSF_FRAME_TYPE_BATTERY: u8 = 0x08;
pub const CRSF_FRAME_TYPE_BAROMETRIC_ALTITUDE: u8 = 0x09;
pub const CRSF_FRAME_TYPE_HEARTBEAT: u8 = 0x0B;
pub const CRSF_FRAME_TYPE_BAROMETER: u8 = 0x11;
pub const CRSF_FRAME_TYPE_MAGNETOMETER: u8 = 0x12;
pub const CRSF_FRAME_TYPE_ATTITUDE: u8 = 0x1E;
pub const CRSF_FRAME_TYPE_FLIGHT_MODE: u8 = 0x21;
pub const CRSF_FRAME_TYPE_DEVICE_INFO: u8 = 0x29;
pub const CRSF_FRAME_TYPE_MSP_RESPONSE: u8 = 0x7B;
pub const CRSF_FRAME_TYPE_RANGE: u8 = 0x7C;

const CRSF_FRAME_LENGTH_MIN: usize = 2;
const CRSF_FRAME_LENGTH_MAX: usize = 62;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CrsfFrame {
    address: u8,
    frame_type: u8,
    payload: Vec<u8>,
    raw: Vec<u8>,
}

impl CrsfFrame {
    pub fn from_hex(value: &str) -> Result<Self, CrsfError> {
        let mut raw = Vec::new();
        for (position, byte) in value.split_ascii_whitespace().enumerate() {
            let value = u8::from_str_radix(byte, 16).map_err(|_| CrsfError::InvalidHexByte {
                position: position + 1,
                value: byte.to_owned(),
            })?;
            raw.push(value);
        }
        Self::try_from(raw.as_slice())
    }

    #[must_use]
    pub const fn address(&self) -> u8 {
        self.address
    }

    #[must_use]
    pub const fn frame_type(&self) -> u8 {
        self.frame_type
    }

    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    #[must_use]
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    #[must_use]
    pub fn raw_hex(&self) -> String {
        self.raw
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn telemetry(&self) -> Result<CrsfTelemetry, CrsfError> {
        match self.frame_type {
            CRSF_FRAME_TYPE_GPS => {
                require_payload(self, 15)?;
                Ok(CrsfTelemetry::Gps {
                    latitude_degrees: f64::from(read_i32(&self.payload[0..4])) / 10_000_000.0,
                    longitude_degrees: f64::from(read_i32(&self.payload[4..8])) / 10_000_000.0,
                    ground_speed_kmh: f32::from(read_u16(&self.payload[8..10])) / 10.0,
                    heading_degrees: f32::from(read_u16(&self.payload[10..12])) / 100.0,
                    altitude_metres: i32::from(read_u16(&self.payload[12..14])) - 1000,
                    satellites: self.payload[14],
                })
            }
            CRSF_FRAME_TYPE_VARIO => {
                require_payload(self, 2)?;
                Ok(CrsfTelemetry::Vario {
                    vertical_speed_ms: f32::from(read_i16(&self.payload[0..2])) / 100.0,
                })
            }
            CRSF_FRAME_TYPE_BATTERY => {
                require_payload(self, 8)?;
                let capacity_mah = (u32::from(self.payload[4]) << 16)
                    | (u32::from(self.payload[5]) << 8)
                    | u32::from(self.payload[6]);
                Ok(CrsfTelemetry::Battery {
                    voltage_v: f32::from(read_u16(&self.payload[0..2])) / 10.0,
                    current_a: f32::from(read_u16(&self.payload[2..4])) / 10.0,
                    capacity_mah,
                    remaining_percent: self.payload[7],
                })
            }
            CRSF_FRAME_TYPE_BAROMETRIC_ALTITUDE => {
                require_payload(self, 3)?;
                let packed_altitude = read_u16(&self.payload[0..2]);
                let altitude_metres = if packed_altitude & 0x8000 == 0 {
                    (f32::from(packed_altitude) - 10_000.0) / 10.0
                } else {
                    f32::from(packed_altitude & 0x7FFF)
                };
                let packed_vertical_speed = self.payload[2] as i8;
                let direction = f32::from(packed_vertical_speed.signum());
                let vertical_speed_ms = direction
                    * (f32::exp(f32::from(packed_vertical_speed.unsigned_abs()) * 0.026) - 1.0);
                Ok(CrsfTelemetry::BarometricAltitude {
                    altitude_metres,
                    vertical_speed_ms,
                })
            }
            CRSF_FRAME_TYPE_HEARTBEAT => Ok(CrsfTelemetry::Heartbeat),
            CRSF_FRAME_TYPE_BAROMETER => {
                require_payload(self, 8)?;
                Ok(CrsfTelemetry::Barometer {
                    pressure_pa: read_i32(&self.payload[0..4]),
                    temperature_c: read_i32(&self.payload[4..8]) as f32 / 100.0,
                })
            }
            CRSF_FRAME_TYPE_MAGNETOMETER => {
                require_payload(self, 6)?;
                Ok(CrsfTelemetry::Magnetometer {
                    x: read_i16(&self.payload[0..2]),
                    y: read_i16(&self.payload[2..4]),
                    z: read_i16(&self.payload[4..6]),
                })
            }
            CRSF_FRAME_TYPE_ATTITUDE => {
                require_payload(self, 6)?;
                Ok(CrsfTelemetry::Attitude {
                    pitch_radians: f32::from(read_i16(&self.payload[0..2])) / 10_000.0,
                    roll_radians: f32::from(read_i16(&self.payload[2..4])) / 10_000.0,
                    yaw_radians: f32::from(read_i16(&self.payload[4..6])) / 10_000.0,
                })
            }
            CRSF_FRAME_TYPE_FLIGHT_MODE => {
                let end = self
                    .payload
                    .iter()
                    .position(|byte| *byte == 0)
                    .unwrap_or(self.payload.len());
                Ok(CrsfTelemetry::FlightMode(
                    String::from_utf8_lossy(&self.payload[..end]).into_owned(),
                ))
            }
            CRSF_FRAME_TYPE_DEVICE_INFO => Ok(CrsfTelemetry::DeviceInfo),
            CRSF_FRAME_TYPE_MSP_RESPONSE => Ok(CrsfTelemetry::MspResponse),
            CRSF_FRAME_TYPE_RANGE => {
                require_payload(self, 12)?;
                let version = self.payload[2];
                if version != 1 {
                    return Err(CrsfError::UnsupportedRangeVersion { version });
                }
                let valid_mask = self.payload[3];
                Ok(CrsfTelemetry::Range {
                    front_metres: read_range(&self.payload[4..6], valid_mask, 0),
                    back_metres: read_range(&self.payload[6..8], valid_mask, 1),
                    left_metres: read_range(&self.payload[8..10], valid_mask, 2),
                    right_metres: read_range(&self.payload[10..12], valid_mask, 3),
                })
            }
            frame_type => Ok(CrsfTelemetry::Unknown { frame_type }),
        }
    }
}

impl TryFrom<&[u8]> for CrsfFrame {
    type Error = CrsfError;

    fn try_from(raw: &[u8]) -> Result<Self, Self::Error> {
        if !(4..=CRSF_FRAME_SIZE_MAX).contains(&raw.len()) {
            return Err(CrsfError::FrameSize { actual: raw.len() });
        }

        let declared = usize::from(raw[1]);
        if !(CRSF_FRAME_LENGTH_MIN..=CRSF_FRAME_LENGTH_MAX).contains(&declared) {
            return Err(CrsfError::DeclaredLengthOutOfRange { declared });
        }
        let expected = declared + 2;
        if raw.len() != expected {
            return Err(CrsfError::LengthMismatch {
                declared: expected,
                actual: raw.len(),
            });
        }

        let expected_crc = raw[raw.len() - 1];
        let actual_crc = crc8_dvb_s2(&raw[2..raw.len() - 1]);
        if actual_crc != expected_crc {
            return Err(CrsfError::CrcMismatch {
                expected: expected_crc,
                actual: actual_crc,
            });
        }

        Ok(Self {
            address: raw[0],
            frame_type: raw[2],
            payload: raw[3..raw.len() - 1].to_vec(),
            raw: raw.to_vec(),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CrsfTelemetry {
    Gps {
        latitude_degrees: f64,
        longitude_degrees: f64,
        ground_speed_kmh: f32,
        heading_degrees: f32,
        altitude_metres: i32,
        satellites: u8,
    },
    Vario {
        vertical_speed_ms: f32,
    },
    Battery {
        voltage_v: f32,
        current_a: f32,
        capacity_mah: u32,
        remaining_percent: u8,
    },
    BarometricAltitude {
        altitude_metres: f32,
        vertical_speed_ms: f32,
    },
    Heartbeat,
    Barometer {
        pressure_pa: i32,
        temperature_c: f32,
    },
    Magnetometer {
        x: i16,
        y: i16,
        z: i16,
    },
    Attitude {
        pitch_radians: f32,
        roll_radians: f32,
        yaw_radians: f32,
    },
    FlightMode(String),
    DeviceInfo,
    MspResponse,
    Range {
        front_metres: Option<f32>,
        back_metres: Option<f32>,
        left_metres: Option<f32>,
        right_metres: Option<f32>,
    },
    Unknown {
        frame_type: u8,
    },
}

impl CrsfTelemetry {
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Gps { .. } => "GPS",
            Self::Vario { .. } => "Vario",
            Self::Battery { .. } => "Battery",
            Self::BarometricAltitude { .. } => "Barometric altitude",
            Self::Heartbeat => "Heartbeat",
            Self::Barometer { .. } => "Barometer",
            Self::Magnetometer { .. } => "Magnetometer",
            Self::Attitude { .. } => "Attitude",
            Self::FlightMode(_) => "Flight mode",
            Self::DeviceInfo => "Device info",
            Self::MspResponse => "MSP response",
            Self::Range { .. } => "Range",
            Self::Unknown { .. } => "Unknown",
        }
    }
}

fn require_payload(frame: &CrsfFrame, expected: usize) -> Result<(), CrsfError> {
    if frame.payload.len() == expected {
        Ok(())
    } else {
        Err(CrsfError::PayloadLength {
            frame_type: frame.frame_type,
            expected,
            actual: frame.payload.len(),
        })
    }
}

fn read_u16(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[0], bytes[1]])
}

fn read_i16(bytes: &[u8]) -> i16 {
    i16::from_be_bytes([bytes[0], bytes[1]])
}

fn read_i32(bytes: &[u8]) -> i32 {
    i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn read_range(bytes: &[u8], valid_mask: u8, index: u8) -> Option<f32> {
    let millimetres = read_u16(bytes);
    if valid_mask & (1 << index) == 0 || millimetres == u16::MAX {
        None
    } else {
        Some(f32::from(millimetres) / 1000.0)
    }
}

#[must_use]
pub fn crc8_dvb_s2(bytes: &[u8]) -> u8 {
    let mut crc = 0_u8;
    for byte in bytes {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0xD5
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CrsfError {
    #[error("CRSF hex byte {position} is invalid: {value}")]
    InvalidHexByte { position: usize, value: String },

    #[error("CRSF frame has {actual} bytes; expected 4-{CRSF_FRAME_SIZE_MAX}")]
    FrameSize { actual: usize },

    #[error("CRSF declared length {declared} is outside 2-62")]
    DeclaredLengthOutOfRange { declared: usize },

    #[error("CRSF declared total length {declared}, received {actual} bytes")]
    LengthMismatch { declared: usize, actual: usize },

    #[error("CRSF CRC mismatch: frame has 0x{expected:02X}, calculated 0x{actual:02X}")]
    CrcMismatch { expected: u8, actual: u8 },

    #[error("CRSF frame type 0x{frame_type:02X} payload has {actual} bytes; expected {expected}")]
    PayloadLength {
        frame_type: u8,
        expected: usize,
        actual: usize,
    },

    #[error("CRSF range telemetry version {version} is unsupported; expected version 1")]
    UnsupportedRangeVersion { version: u8 },
}
