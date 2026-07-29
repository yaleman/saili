use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use prost::Message;
use snow::{Builder, TransportState, params::NoiseParams};
use thiserror::Error;

use crate::{RC_CHANNEL_COUNT, RcChannels};

const DEFAULT_PORT: u16 = 6053;
const NOISE_PROTOCOL: &str = "Noise_NNpsk0_25519_ChaChaPoly_SHA256";
const NOISE_PROLOGUE: &[u8] = b"NoiseAPIInit\0\0";
const NOISE_MARKER: u8 = 0x01;
const MAX_FRAME_SIZE: usize = u16::MAX as usize;
const API_VERSION_MAJOR: u32 = 1;
const API_VERSION_MINOR: u32 = 14;

const HELLO_REQUEST: u16 = 1;
const HELLO_RESPONSE: u16 = 2;
const DISCONNECT_REQUEST: u16 = 5;
const DISCONNECT_RESPONSE: u16 = 6;
const PING_REQUEST: u16 = 7;
const PING_RESPONSE: u16 = 8;
const LIST_ENTITIES_REQUEST: u16 = 11;
const LIST_ENTITIES_DONE_RESPONSE: u16 = 19;
const LIST_ENTITIES_SERVICES_RESPONSE: u16 = 41;
const EXECUTE_SERVICE_REQUEST: u16 = 42;
const EXECUTE_SERVICE_RESPONSE: u16 = 131;

const RC_ACTION_NAME: &str = "set_rc_channels";
const SUPPORTS_RESPONSE_STATUS: u32 = 100;
const RC_ACTION_ARGUMENTS: [&str; RC_CHANNEL_COUNT] = [
    "roll_us",
    "pitch_us",
    "throttle_us",
    "yaw_us",
    "aux1_us",
    "aux2_us",
    "aux3_us",
    "aux4_us",
    "aux5_us",
    "aux6_us",
    "aux7_us",
    "aux8_us",
    "aux9_us",
    "aux10_us",
    "aux11_us",
    "aux12_us",
];
const SERVICE_ARGUMENT_TYPE_INT: i32 = 1;

pub struct EspHomeRcClient {
    stream: TcpStream,
    transport: TransportState,
    action_key: u32,
    next_call_id: u32,
    server: ServerIdentity,
}

impl EspHomeRcClient {
    pub fn connect(
        address: &str,
        encryption_key: &str,
        timeout: Duration,
    ) -> Result<Self, EspHomeError> {
        let address = with_default_port(address);
        let mut resolved = address
            .to_socket_addrs()
            .map_err(|source| EspHomeError::Resolve {
                address: address.clone(),
                source,
            })?;
        let socket_address = resolved.next().ok_or_else(|| EspHomeError::NoAddress {
            address: address.clone(),
        })?;
        let mut stream =
            TcpStream::connect_timeout(&socket_address, timeout).map_err(|source| {
                EspHomeError::Connect {
                    address: address.clone(),
                    source,
                }
            })?;
        stream
            .set_nodelay(true)
            .map_err(EspHomeError::ConfigureSocket)?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(EspHomeError::ConfigureSocket)?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(EspHomeError::ConfigureSocket)?;

        let key = decode_encryption_key(encryption_key)?;
        let (transport, noise_identity) = noise_handshake(&mut stream, &key)?;

        let mut client = Self {
            stream,
            transport,
            action_key: 0,
            next_call_id: 1,
            server: ServerIdentity {
                name: noise_identity.name,
                mac_address: noise_identity.mac_address,
                version: String::new(),
            },
        };

        client.send_message(
            HELLO_REQUEST,
            &proto::HelloRequest {
                client_info: format!("saili {}", env!("CARGO_PKG_VERSION")),
                api_version_major: API_VERSION_MAJOR,
                api_version_minor: API_VERSION_MINOR,
            },
        )?;
        let response = client.read_until(HELLO_RESPONSE, timeout)?;
        let hello = proto::HelloResponse::decode(response.payload.as_slice())?;
        if hello.api_version_major != API_VERSION_MAJOR {
            return Err(EspHomeError::UnsupportedApiVersion {
                major: hello.api_version_major,
                minor: hello.api_version_minor,
            });
        }
        if !hello.name.is_empty() {
            client.server.name = hello.name;
        }
        client.server.version = hello.server_info;

        client.send_empty(LIST_ENTITIES_REQUEST)?;
        client.action_key = client.discover_action(timeout)?;

        Ok(client)
    }

    #[must_use]
    pub const fn server(&self) -> &ServerIdentity {
        &self.server
    }

    pub fn send_channels(
        &mut self,
        channels: RcChannels,
        timeout: Duration,
    ) -> Result<ActionAcknowledgement, EspHomeError> {
        self.stream
            .set_read_timeout(Some(timeout))
            .map_err(EspHomeError::ConfigureSocket)?;
        let call_id = self.next_call_id;
        self.next_call_id = self.next_call_id.wrapping_add(1).max(1);

        let args = channels
            .values()
            .iter()
            .copied()
            .map(|value| proto::ExecuteServiceArgument {
                int_value: i32::from(value),
            })
            .collect();
        let request = proto::ExecuteServiceRequest {
            key: self.action_key,
            args,
            call_id,
            return_response: false,
        };

        let started = Instant::now();
        self.send_message(EXECUTE_SERVICE_REQUEST, &request)?;

        loop {
            let response = match self.read_message() {
                Ok(response) => response,
                Err(EspHomeError::Io(source)) if is_timeout(&source) => {
                    return Err(EspHomeError::ActionTimeout { timeout });
                }
                Err(error) => return Err(error),
            };
            match response.message_type {
                EXECUTE_SERVICE_RESPONSE => {
                    let response =
                        proto::ExecuteServiceResponse::decode(response.payload.as_slice())?;
                    if response.call_id != call_id {
                        continue;
                    }
                    if !response.success {
                        return Err(EspHomeError::ActionRejected {
                            message: response.error_message,
                        });
                    }
                    return Ok(ActionAcknowledgement {
                        round_trip: started.elapsed(),
                    });
                }
                PING_REQUEST => self.send_empty(PING_RESPONSE)?,
                DISCONNECT_REQUEST => {
                    self.send_empty(DISCONNECT_RESPONSE)?;
                    return Err(EspHomeError::Disconnected);
                }
                _ => {}
            }

            if started.elapsed() >= timeout {
                return Err(EspHomeError::ActionTimeout { timeout });
            }
        }
    }

    fn discover_action(&mut self, timeout: Duration) -> Result<u32, EspHomeError> {
        self.stream
            .set_read_timeout(Some(timeout))
            .map_err(EspHomeError::ConfigureSocket)?;
        let started = Instant::now();
        let mut matching_action = None;

        loop {
            let response = match self.read_message() {
                Ok(response) => response,
                Err(EspHomeError::Io(source)) if is_timeout(&source) => {
                    return Err(EspHomeError::DiscoveryTimeout { timeout });
                }
                Err(error) => return Err(error),
            };
            match response.message_type {
                LIST_ENTITIES_SERVICES_RESPONSE => {
                    let service =
                        proto::ListEntitiesServicesResponse::decode(response.payload.as_slice())?;
                    if service.name == RC_ACTION_NAME {
                        validate_action_schema(&service)?;
                        matching_action = Some(service.key);
                    }
                }
                LIST_ENTITIES_DONE_RESPONSE => {
                    return matching_action.ok_or(EspHomeError::ActionNotFound);
                }
                PING_REQUEST => self.send_empty(PING_RESPONSE)?,
                DISCONNECT_REQUEST => {
                    self.send_empty(DISCONNECT_RESPONSE)?;
                    return Err(EspHomeError::Disconnected);
                }
                _ => {}
            }

            if started.elapsed() >= timeout {
                return Err(EspHomeError::DiscoveryTimeout { timeout });
            }
        }
    }

    fn send_empty(&mut self, message_type: u16) -> Result<(), EspHomeError> {
        self.send_payload(message_type, &[])
    }

    fn send_message<M: Message>(
        &mut self,
        message_type: u16,
        message: &M,
    ) -> Result<(), EspHomeError> {
        self.send_payload(message_type, &message.encode_to_vec())
    }

    fn send_payload(&mut self, message_type: u16, payload: &[u8]) -> Result<(), EspHomeError> {
        let payload_length =
            u16::try_from(payload.len()).map_err(|_| EspHomeError::FrameTooLarge {
                size: payload.len(),
            })?;
        let mut plaintext = Vec::with_capacity(payload.len() + 4);
        plaintext.extend_from_slice(&message_type.to_be_bytes());
        plaintext.extend_from_slice(&payload_length.to_be_bytes());
        plaintext.extend_from_slice(payload);

        let mut encrypted = vec![0; plaintext.len() + 16];
        let encrypted_length = self
            .transport
            .write_message(&plaintext, &mut encrypted)
            .map_err(EspHomeError::Noise)?;
        encrypted.truncate(encrypted_length);
        write_frame(&mut self.stream, &encrypted)
    }

    fn read_until(
        &mut self,
        expected_type: u16,
        timeout: Duration,
    ) -> Result<WireMessage, EspHomeError> {
        self.stream
            .set_read_timeout(Some(timeout))
            .map_err(EspHomeError::ConfigureSocket)?;
        let started = Instant::now();

        loop {
            let message = match self.read_message() {
                Ok(message) => message,
                Err(EspHomeError::Io(source)) if is_timeout(&source) => {
                    return Err(EspHomeError::MessageTimeout {
                        message_type: expected_type,
                        timeout,
                    });
                }
                Err(error) => return Err(error),
            };
            match message.message_type {
                message_type if message_type == expected_type => return Ok(message),
                PING_REQUEST => self.send_empty(PING_RESPONSE)?,
                DISCONNECT_REQUEST => {
                    self.send_empty(DISCONNECT_RESPONSE)?;
                    return Err(EspHomeError::Disconnected);
                }
                _ => {}
            }

            if started.elapsed() >= timeout {
                return Err(EspHomeError::MessageTimeout {
                    message_type: expected_type,
                    timeout,
                });
            }
        }
    }

    fn read_message(&mut self) -> Result<WireMessage, EspHomeError> {
        let encrypted = read_frame(&mut self.stream)?;
        let mut plaintext = vec![0; encrypted.len()];
        let plaintext_length = self
            .transport
            .read_message(&encrypted, &mut plaintext)
            .map_err(EspHomeError::Noise)?;
        plaintext.truncate(plaintext_length);

        if plaintext.len() < 4 {
            return Err(EspHomeError::MalformedMessage {
                reason: MalformedMessageReason::HeaderTooShort,
            });
        }

        let message_type = u16::from_be_bytes([plaintext[0], plaintext[1]]);
        let declared_length = usize::from(u16::from_be_bytes([plaintext[2], plaintext[3]]));
        let payload = plaintext.split_off(4);
        if payload.len() != declared_length {
            return Err(EspHomeError::MalformedMessage {
                reason: MalformedMessageReason::LengthMismatch {
                    declared: declared_length,
                    actual: payload.len(),
                },
            });
        }

        Ok(WireMessage {
            message_type,
            payload,
        })
    }
}

fn noise_handshake(
    stream: &mut TcpStream,
    key: &[u8; 32],
) -> Result<(TransportState, NoiseIdentity), EspHomeError> {
    let parameters: NoiseParams = NOISE_PROTOCOL.parse().map_err(EspHomeError::Noise)?;
    let mut handshake = Builder::new(parameters)
        .psk(0, key)
        .map_err(EspHomeError::Noise)?
        .prologue(NOISE_PROLOGUE)
        .map_err(EspHomeError::Noise)?
        .build_initiator()
        .map_err(EspHomeError::Noise)?;

    let mut handshake_message = [0; 128];
    let handshake_length = handshake
        .write_message(&[], &mut handshake_message)
        .map_err(EspHomeError::Noise)?;

    stream.write_all(&[NOISE_MARKER, 0, 0])?;
    let mut client_hello = Vec::with_capacity(handshake_length + 1);
    client_hello.push(0);
    client_hello.extend_from_slice(&handshake_message[..handshake_length]);
    write_frame(stream, &client_hello)?;

    let server_hello = read_frame(stream)?;
    let identity = parse_server_hello(&server_hello)?;

    let response = read_frame(stream)?;
    let (&status, handshake_payload) =
        response
            .split_first()
            .ok_or(EspHomeError::HandshakeRejected {
                message: "empty handshake response".to_owned(),
            })?;
    if status != 0 {
        return Err(EspHomeError::HandshakeRejected {
            message: String::from_utf8_lossy(handshake_payload).into_owned(),
        });
    }

    let mut output = [0; 128];
    handshake
        .read_message(handshake_payload, &mut output)
        .map_err(EspHomeError::Noise)?;
    let transport = handshake
        .into_transport_mode()
        .map_err(EspHomeError::Noise)?;

    Ok((transport, identity))
}

fn parse_server_hello(frame: &[u8]) -> Result<NoiseIdentity, EspHomeError> {
    let (&protocol, details) = frame
        .split_first()
        .ok_or(EspHomeError::MalformedServerHello)?;
    if protocol != 1 {
        return Err(EspHomeError::UnsupportedNoiseProtocol { protocol });
    }

    let mut fields = details.split(|byte| *byte == 0);
    let name = fields
        .next()
        .map(|field| String::from_utf8_lossy(field).into_owned())
        .unwrap_or_default();
    let mac_address = fields
        .next()
        .map(|field| String::from_utf8_lossy(field).into_owned())
        .unwrap_or_default();

    Ok(NoiseIdentity { name, mac_address })
}

fn validate_action_schema(
    service: &proto::ListEntitiesServicesResponse,
) -> Result<(), EspHomeError> {
    if service.supports_response != SUPPORTS_RESPONSE_STATUS {
        return Err(EspHomeError::ActionSchemaMismatch {
            reason: ActionSchemaMismatch::ResponseMode {
                expected: SUPPORTS_RESPONSE_STATUS,
                actual: service.supports_response,
            },
        });
    }

    if service.args.len() != RC_ACTION_ARGUMENTS.len() {
        return Err(EspHomeError::ActionSchemaMismatch {
            reason: ActionSchemaMismatch::ArgumentCount {
                expected: RC_ACTION_ARGUMENTS.len(),
                actual: service.args.len(),
            },
        });
    }

    for (index, (argument, expected_name)) in
        service.args.iter().zip(RC_ACTION_ARGUMENTS).enumerate()
    {
        if argument.name != expected_name {
            return Err(EspHomeError::ActionSchemaMismatch {
                reason: ActionSchemaMismatch::ArgumentName {
                    position: index + 1,
                    expected: expected_name,
                    actual: argument.name.clone(),
                },
            });
        }
        if argument.argument_type != SERVICE_ARGUMENT_TYPE_INT {
            return Err(EspHomeError::ActionSchemaMismatch {
                reason: ActionSchemaMismatch::ArgumentType {
                    name: argument.name.clone(),
                    expected: SERVICE_ARGUMENT_TYPE_INT,
                    actual: argument.argument_type,
                },
            });
        }
    }

    Ok(())
}

fn is_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

fn decode_encryption_key(value: &str) -> Result<[u8; 32], EspHomeError> {
    let decoded = STANDARD
        .decode(value.trim())
        .map_err(EspHomeError::InvalidEncryptionKey)?;
    decoded
        .try_into()
        .map_err(|decoded: Vec<u8>| EspHomeError::EncryptionKeyLength {
            actual: decoded.len(),
        })
}

fn with_default_port(address: &str) -> String {
    let address = address.trim();
    if address
        .rsplit_once(':')
        .is_some_and(|(_, port)| port.parse::<u16>().is_ok())
    {
        address.to_owned()
    } else {
        format!("{address}:{DEFAULT_PORT}")
    }
}

fn write_frame(stream: &mut TcpStream, payload: &[u8]) -> Result<(), EspHomeError> {
    let length = u16::try_from(payload.len()).map_err(|_| EspHomeError::FrameTooLarge {
        size: payload.len(),
    })?;
    let [high, low] = length.to_be_bytes();
    stream.write_all(&[NOISE_MARKER, high, low])?;
    stream.write_all(payload)?;
    Ok(())
}

fn read_frame(stream: &mut TcpStream) -> Result<Vec<u8>, EspHomeError> {
    let mut header = [0; 3];
    stream.read_exact(&mut header)?;
    if header[0] != NOISE_MARKER {
        return Err(EspHomeError::UnexpectedFrameMarker { marker: header[0] });
    }
    let size = usize::from(u16::from_be_bytes([header[1], header[2]]));
    if size > MAX_FRAME_SIZE {
        return Err(EspHomeError::FrameTooLarge { size });
    }
    let mut payload = vec![0; size];
    stream.read_exact(&mut payload)?;
    Ok(payload)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerIdentity {
    pub name: String,
    pub mac_address: String,
    pub version: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActionAcknowledgement {
    pub round_trip: Duration,
}

struct NoiseIdentity {
    name: String,
    mac_address: String,
}

struct WireMessage {
    message_type: u16,
    payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ActionSchemaMismatch {
    #[error("action response mode should be status ({expected}), device advertised {actual}")]
    ResponseMode { expected: u32, actual: u32 },

    #[error("expected {expected} arguments, device advertised {actual}")]
    ArgumentCount { expected: usize, actual: usize },

    #[error("argument {position} should be {expected}, device advertised {actual}")]
    ArgumentName {
        position: usize,
        expected: &'static str,
        actual: String,
    },

    #[error("argument {name} should have type {expected}, device advertised {actual}")]
    ArgumentType {
        name: String,
        expected: i32,
        actual: i32,
    },
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum MalformedMessageReason {
    #[error("decrypted message header is shorter than four bytes")]
    HeaderTooShort,

    #[error("declared payload length {declared} does not match actual length {actual}")]
    LengthMismatch { declared: usize, actual: usize },
}

#[derive(Debug, Error)]
pub enum EspHomeError {
    #[error("could not resolve ESPHome address {address}")]
    Resolve {
        address: String,
        #[source]
        source: io::Error,
    },

    #[error("ESPHome address {address} did not resolve to an IP address")]
    NoAddress { address: String },

    #[error("could not connect to ESPHome at {address}")]
    Connect {
        address: String,
        #[source]
        source: io::Error,
    },

    #[error("could not configure the ESPHome TCP socket")]
    ConfigureSocket(#[source] io::Error),

    #[error("the ESPHome API encryption key is not valid base64")]
    InvalidEncryptionKey(#[source] base64::DecodeError),

    #[error("the ESPHome API encryption key decoded to {actual} bytes instead of 32")]
    EncryptionKeyLength { actual: usize },

    #[error("ESPHome Noise protocol failure")]
    Noise(#[source] snow::Error),

    #[error("ESPHome rejected the encrypted handshake: {message}")]
    HandshakeRejected { message: String },

    #[error("ESPHome sent an empty server hello")]
    MalformedServerHello,

    #[error("ESPHome selected unsupported Noise protocol {protocol}")]
    UnsupportedNoiseProtocol { protocol: u8 },

    #[error("ESPHome frame marker 0x{marker:02x} is not the encrypted API marker")]
    UnexpectedFrameMarker { marker: u8 },

    #[error("ESPHome frame of {size} bytes is too large")]
    FrameTooLarge { size: usize },

    #[error("ESPHome sent a malformed message: {reason}")]
    MalformedMessage { reason: MalformedMessageReason },

    #[error("ESPHome API version {major}.{minor} is not supported")]
    UnsupportedApiVersion { major: u32, minor: u32 },

    #[error("ESPHome did not advertise the set_rc_channels action")]
    ActionNotFound,

    #[error("ESPHome set_rc_channels schema differs from the bridge config: {reason}")]
    ActionSchemaMismatch { reason: ActionSchemaMismatch },

    #[error("ESPHome action discovery timed out after {timeout:?}")]
    DiscoveryTimeout { timeout: Duration },

    #[error("ESPHome message type {message_type} timed out after {timeout:?}")]
    MessageTimeout {
        message_type: u16,
        timeout: Duration,
    },

    #[error("ESPHome action acknowledgement timed out after {timeout:?}")]
    ActionTimeout { timeout: Duration },

    #[error("ESPHome rejected the RC command: {message}")]
    ActionRejected { message: String },

    #[error("ESPHome disconnected")]
    Disconnected,

    #[error("ESPHome protobuf could not be decoded")]
    Decode(#[from] prost::DecodeError),

    #[error("ESPHome network I/O failed")]
    Io(#[from] io::Error),
}

mod proto {
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct HelloRequest {
        #[prost(string, tag = "1")]
        pub client_info: String,
        #[prost(uint32, tag = "2")]
        pub api_version_major: u32,
        #[prost(uint32, tag = "3")]
        pub api_version_minor: u32,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct HelloResponse {
        #[prost(uint32, tag = "1")]
        pub api_version_major: u32,
        #[prost(uint32, tag = "2")]
        pub api_version_minor: u32,
        #[prost(string, tag = "3")]
        pub server_info: String,
        #[prost(string, tag = "4")]
        pub name: String,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ListEntitiesServicesArgument {
        #[prost(string, tag = "1")]
        pub name: String,
        #[prost(enumeration = "ServiceArgumentType", tag = "2")]
        pub argument_type: i32,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ListEntitiesServicesResponse {
        #[prost(string, tag = "1")]
        pub name: String,
        #[prost(fixed32, tag = "2")]
        pub key: u32,
        #[prost(message, repeated, tag = "3")]
        pub args: Vec<ListEntitiesServicesArgument>,
        #[prost(uint32, tag = "4")]
        pub supports_response: u32,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ExecuteServiceArgument {
        #[prost(sint32, tag = "5")]
        pub int_value: i32,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ExecuteServiceRequest {
        #[prost(fixed32, tag = "1")]
        pub key: u32,
        #[prost(message, repeated, tag = "2")]
        pub args: Vec<ExecuteServiceArgument>,
        #[prost(uint32, tag = "3")]
        pub call_id: u32,
        #[prost(bool, tag = "4")]
        pub return_response: bool,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ExecuteServiceResponse {
        #[prost(uint32, tag = "1")]
        pub call_id: u32,
        #[prost(bool, tag = "2")]
        pub success: bool,
        #[prost(string, tag = "3")]
        pub error_message: String,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, prost::Enumeration)]
    #[repr(i32)]
    pub enum ServiceArgumentType {
        Bool = 0,
        Int = 1,
        Float = 2,
        String = 3,
        BoolArray = 4,
        IntArray = 5,
        FloatArray = 6,
        StringArray = 7,
    }
}
