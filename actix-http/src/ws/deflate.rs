//! WebSocket permessage-deflate compression implementation.

use std::convert::Infallible;

use bytes::Bytes;
pub use flate2::Compression as DeflateCompressionLevel;

use super::{OpCode, ProtocolError, RsvBits};
use crate::header::{HeaderName, HeaderValue, TryIntoHeaderPair, SEC_WEBSOCKET_EXTENSIONS};

// NOTE: according to [RFC 7692 §7.1.2.1] window bit size should be within 8..=15
// but we have to limit the range to 9..=15 because [flate2] only supports window bit within 9..=15.
//
// [RFC 6792 §7.1.2.1]: https://datatracker.ietf.org/doc/html/rfc7692#section-7.1.2.1
// [flate2]: https://docs.rs/flate2/latest/flate2/struct.Compress.html#method.new_with_window_bits
const MAX_WINDOW_BITS_RANGE: std::ops::RangeInclusive<u8> = 9..=15;
const DEFAULT_WINDOW_BITS: u8 = 15;

const BUF_SIZE: usize = 2048;

pub(super) const RSV_BIT_DEFLATE_FLAG: RsvBits = RsvBits::RSV1;

/// DEFLATE compression related handshake errors.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DeflateHandshakeError {
    /// Unknown extension parameter given.
    UnknownWebSocketParameters,

    /// Duplicate parameter found in single extension statement.
    DuplicateParameter(&'static str),

    /// Max window bits size out of range. Should be in 9..=15
    MaxWindowBitsOutOfRange,

    /// Multiple `permessage-deflate` statements found but failed to negotiate any.
    NoSuitableConfigurationFound,
}

impl std::fmt::Display for DeflateHandshakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownWebSocketParameters => {
                write!(f, "Unknown WebSocket `permessage-deflate` parameters.")
            }
            Self::DuplicateParameter(p) => {
                write!(f, "Duplicate WebSocket `permessage-deflate` parameter: {p}")
            }
            Self::MaxWindowBitsOutOfRange => write!(
                f,
                "Max window bits out of range. ({} to {} expected)",
                MAX_WINDOW_BITS_RANGE.start(),
                MAX_WINDOW_BITS_RANGE.end()
            ),
            Self::NoSuitableConfigurationFound => write!(
                f,
                "No suitable WebSocket `permedia-deflate` parameter configurations found."
            ),
        }
    }
}

impl std::error::Error for DeflateHandshakeError {}

/// Maximum size of client's DEFLATE sliding window.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ClientMaxWindowBits {
    /// Unspecified. Indicates client will follow server configuration.
    NotSpecified,
    /// Specified size of client's DEFLATE sliding window size in bits, between 9 and 15.
    Specified(u8),
}

/// Per-session DEFLATE configuration parameter.
///
/// It can be used both client and server side.
/// At client side, it can be used to pass desired configuration to server.
/// At server side, negotiated parameter will be sent to client with this.
/// This can be represented in HTTP header form as it implements [`TryIntoHeaderPair`] trait.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct DeflateSessionParameters {
    /// Disallow server from take over context.
    pub server_no_context_takeover: bool,
    /// Disallow client from take over context.
    pub client_no_context_takeover: bool,
    /// Maximum size of server's DEFLATE sliding window in bits, between 9 and 15.
    pub server_max_window_bits: Option<u8>,
    /// Maximum size of client's DEFLATE sliding window.
    pub client_max_window_bits: Option<ClientMaxWindowBits>,
}

impl TryIntoHeaderPair for DeflateSessionParameters {
    type Error = Infallible;

    fn try_into_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error> {
        let mut response_extension = vec!["permessage-deflate".to_owned()];

        if self.server_no_context_takeover {
            response_extension.push("server_no_context_takeover".to_owned());
        }
        if self.client_no_context_takeover {
            response_extension.push("client_no_context_takeover".to_owned());
        }
        if let Some(server_max_window_bits) = self.server_max_window_bits {
            response_extension.push(format!("server_max_window_bits={server_max_window_bits}"));
        }
        if let Some(client_max_window_bits) = self.client_max_window_bits {
            match client_max_window_bits {
                ClientMaxWindowBits::NotSpecified => {
                    response_extension.push("client_max_window_bits".to_string());
                }
                ClientMaxWindowBits::Specified(bits) => {
                    response_extension.push(format!("client_max_window_bits={bits}"));
                }
            }
        }

        Ok((
            SEC_WEBSOCKET_EXTENSIONS,
            HeaderValue::from_str(&response_extension.join("; ")).unwrap(),
        ))
    }
}

impl DeflateSessionParameters {
    fn parse<'a>(
        extension_frags: impl Iterator<Item = &'a str>,
    ) -> Result<Self, DeflateHandshakeError> {
        let mut client_max_window_bits = None;
        let mut server_max_window_bits = None;
        let mut client_no_context_takeover = None;
        let mut server_no_context_takeover = None;

        let mut unknown_parameters = vec![];

        for fragment in extension_frags {
            if fragment.is_empty() {
                continue;
            } else if fragment == "client_max_window_bits" {
                if client_max_window_bits.is_some() {
                    return Err(DeflateHandshakeError::DuplicateParameter(
                        "client_max_window_bits",
                    ));
                }
                client_max_window_bits = Some(ClientMaxWindowBits::NotSpecified);
            } else if let Some(value) = fragment.strip_prefix("client_max_window_bits=") {
                if client_max_window_bits.is_some() {
                    return Err(DeflateHandshakeError::DuplicateParameter(
                        "client_max_window_bits",
                    ));
                }
                let bits = value
                    .parse::<u8>()
                    .map_err(|_| DeflateHandshakeError::MaxWindowBitsOutOfRange)?;
                if !MAX_WINDOW_BITS_RANGE.contains(&bits) {
                    return Err(DeflateHandshakeError::MaxWindowBitsOutOfRange);
                }
                client_max_window_bits = Some(ClientMaxWindowBits::Specified(bits));
            } else if let Some(value) = fragment.strip_prefix("server_max_window_bits=") {
                if server_max_window_bits.is_some() {
                    return Err(DeflateHandshakeError::DuplicateParameter(
                        "server_max_window_bits",
                    ));
                }
                let bits = value
                    .parse::<u8>()
                    .map_err(|_| DeflateHandshakeError::MaxWindowBitsOutOfRange)?;
                if !MAX_WINDOW_BITS_RANGE.contains(&bits) {
                    return Err(DeflateHandshakeError::MaxWindowBitsOutOfRange);
                }
                server_max_window_bits = Some(bits);
            } else if fragment == "server_no_context_takeover" {
                if server_no_context_takeover.is_some() {
                    return Err(DeflateHandshakeError::DuplicateParameter(
                        "server_no_context_takeover",
                    ));
                }
                server_no_context_takeover = Some(true);
            } else if fragment == "client_no_context_takeover" {
                if client_no_context_takeover.is_some() {
                    return Err(DeflateHandshakeError::DuplicateParameter(
                        "client_no_context_takeover",
                    ));
                }
                client_no_context_takeover = Some(true);
            } else {
                unknown_parameters.push(fragment.to_owned());
            }
        }

        if !unknown_parameters.is_empty() {
            Err(DeflateHandshakeError::UnknownWebSocketParameters)
        } else {
            Ok(DeflateSessionParameters {
                server_no_context_takeover: server_no_context_takeover.unwrap_or(false),
                client_no_context_takeover: client_no_context_takeover.unwrap_or(false),
                server_max_window_bits,
                client_max_window_bits,
            })
        }
    }

    /// Parse desired parameters from `Sec-WebSocket-Extensions` header.
    /// The result may contain multiple values as it's possible to pass multiple parameters
    /// separated with comma.
    pub fn from_extension_header(header_value: &str) -> Vec<Result<Self, DeflateHandshakeError>> {
        let mut results = vec![];
        for extension in header_value.split(',').map(str::trim) {
            let mut fragments = extension.split(';').map(str::trim);
            if fragments.next() == Some("permessage-deflate") {
                results.push(Self::parse(fragments));
            }
        }

        results
    }

    /// Create compression and decompression context based on the parameter.
    pub fn create_context(
        &self,
        compression_level: Option<DeflateCompressionLevel>,
        is_client_mode: bool,
    ) -> (DeflateCompressionContext, DeflateDecompressionContext) {
        let client_max_window_bits =
            if let Some(ClientMaxWindowBits::Specified(value)) = self.client_max_window_bits {
                value
            } else {
                DEFAULT_WINDOW_BITS
            };
        let server_max_window_bits = self.server_max_window_bits.unwrap_or(DEFAULT_WINDOW_BITS);

        let (remote_no_context_takeover, remote_max_window_bits) = if is_client_mode {
            (self.server_no_context_takeover, server_max_window_bits)
        } else {
            (self.client_no_context_takeover, client_max_window_bits)
        };

        let (local_no_context_takeover, local_max_window_bits) = if is_client_mode {
            (self.client_no_context_takeover, client_max_window_bits)
        } else {
            (self.server_no_context_takeover, server_max_window_bits)
        };

        (
            DeflateCompressionContext::new(
                compression_level,
                remote_no_context_takeover,
                remote_max_window_bits,
            ),
            DeflateDecompressionContext::new(local_no_context_takeover, local_max_window_bits),
        )
    }
}

/// Server-side DEFLATE configuration.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeflateServerConfig {
    /// DEFLATE compression level. See [`flate2::Compression`] for details.
    pub compression_level: Option<DeflateCompressionLevel>,
    /// Disallow server from take over context. Default is false.
    pub server_no_context_takeover: bool,
    /// Disallow client from take over context. Default is false.
    pub client_no_context_takeover: bool,
    /// Maximum size of server's DEFLATE sliding window in bits, between 9 and 15. Default is 15.
    pub server_max_window_bits: Option<u8>,
    /// Maximum size of client's DEFLATE sliding window in bits, between 9 and 15. Default is 15.
    pub client_max_window_bits: Option<u8>,
}

impl DeflateServerConfig {
    /// Negotiate context parameters.
    /// Since parameters from the client may be incompatible with the server configuration,
    /// actual parameters could be adjusted here. Conversion rules are as follows:
    ///
    /// ## server_no_context_takeover
    ///
    /// | Config | Request | Response  |
    /// | ------ | ------- | --------- |
    /// | false  | false   | false     |
    /// | false  | true    | true      |
    /// | true   | false   | true      |
    /// | true   | true    | true      |
    ///
    /// ## client_no_context_takeover
    ///
    /// | Config | Request | Response  |
    /// | ------ | ------- | --------- |
    /// | false  | false   | false     |
    /// | false  | true    | true      |
    /// | true   | false   | true      |
    /// | true   | true    | true      |
    ///
    /// ## server_max_window_bits
    ///
    /// | Config       | Request      | Response |
    /// | ------------ | ------------ | -------- |
    /// | None         | None         | None     |
    /// | None         | 9 <= R <= 15 | R        |
    /// | 9 <= C <= 15 | None         | C        |
    /// | 9 <= C <= 15 | 9 <= R <= C  | R        |
    /// | 9 <= C <= 15 | C <= R <= 15 | C        |
    ///
    /// ## client_max_window_bits
    ///
    /// | Config       | Request      | Response |
    /// | ------------ | ------------ | -------- |
    /// | None         | None         | None     |
    /// | None         | Unspecified  | None     |
    /// | None         | 9 <= R <= 15 | R        |
    /// | 9 <= C <= 15 | None         | None     |
    /// | 9 <= C <= 15 | Unspecified  | C        |
    /// | 9 <= C <= 15 | 9 <= R <= C  | R        |
    /// | 9 <= C <= 15 | C <= R <= 15 | C        |
    pub fn negotiate(&self, params: DeflateSessionParameters) -> DeflateSessionParameters {
        let server_no_context_takeover =
            if self.server_no_context_takeover && !params.server_no_context_takeover {
                true
            } else {
                params.server_no_context_takeover
            };

        let client_no_context_takeover =
            if self.client_no_context_takeover && !params.client_no_context_takeover {
                true
            } else {
                params.client_no_context_takeover
            };

        let server_max_window_bits =
            match (self.server_max_window_bits, params.server_max_window_bits) {
                (None, value) => value,
                (Some(config_value), None) => Some(config_value),
                (Some(config_value), Some(value)) => {
                    if value > config_value {
                        Some(config_value)
                    } else {
                        Some(value)
                    }
                }
            };

        let client_max_window_bits =
            match (self.client_max_window_bits, params.client_max_window_bits) {
                (None, None | Some(ClientMaxWindowBits::NotSpecified)) => None,
                (None, Some(ClientMaxWindowBits::Specified(value))) => Some(value),
                (Some(_), None) => None,
                (Some(config_value), Some(ClientMaxWindowBits::NotSpecified)) => Some(config_value),
                (Some(config_value), Some(ClientMaxWindowBits::Specified(value))) => {
                    if value > config_value {
                        Some(config_value)
                    } else {
                        Some(value)
                    }
                }
            };

        DeflateSessionParameters {
            server_no_context_takeover,
            client_no_context_takeover,
            server_max_window_bits,
            client_max_window_bits: client_max_window_bits.map(ClientMaxWindowBits::Specified),
        }
    }
}

/// DEFLATE decompression context.
#[derive(Debug)]
pub struct DeflateDecompressionContext {
    pub(super) local_no_context_takeover: bool,
    pub(super) local_max_window_bits: u8,

    decompress: flate2::Decompress,

    decode_continuation: bool,
    total_bytes_written: u64,
    total_bytes_read: u64,
}

impl DeflateDecompressionContext {
    pub(super) fn new(local_no_context_takeover: bool, local_max_window_bits: u8) -> Self {
        Self {
            local_no_context_takeover,
            local_max_window_bits,

            decompress: flate2::Decompress::new_with_window_bits(false, local_max_window_bits),

            decode_continuation: false,
            total_bytes_written: 0,
            total_bytes_read: 0,
        }
    }

    pub(super) fn reset_with(
        &mut self,
        local_no_context_takeover: bool,
        local_max_window_bits: u8,
    ) {
        *self = Self::new(local_no_context_takeover, local_max_window_bits);
    }

    pub(super) fn decompress(
        &mut self,
        fin: bool,
        opcode: OpCode,
        rsv: RsvBits,
        payload: Bytes,
    ) -> Result<Bytes, ProtocolError> {
        if !matches!(opcode, OpCode::Text | OpCode::Binary | OpCode::Continue)
            || !rsv.contains(RSV_BIT_DEFLATE_FLAG)
        {
            return Ok(payload);
        }

        if opcode == OpCode::Continue {
            if !self.decode_continuation {
                return Ok(payload);
            }
        } else {
            self.decode_continuation = true;
        }

        let mut output: Vec<u8> = vec![];
        let mut buf = [0u8; BUF_SIZE];

        let mut offset: usize = 0;
        loop {
            let res = if offset >= payload.len() {
                self.decompress
                    .decompress(
                        &[0x00, 0x00, 0xff, 0xff],
                        &mut buf,
                        flate2::FlushDecompress::Finish,
                    )
                    .map_err(|err| {
                        self.reset();
                        ProtocolError::Io(err.into())
                    })?
            } else {
                self.decompress
                    .decompress(&payload[offset..], &mut buf, flate2::FlushDecompress::None)
                    .map_err(|err| {
                        self.reset();
                        ProtocolError::Io(err.into())
                    })?
            };

            let read = self.decompress.total_in() - self.total_bytes_read;
            let written = self.decompress.total_out() - self.total_bytes_written;

            offset += read as usize;
            self.total_bytes_read += read;
            if written > 0 {
                output.extend(buf.iter().take(written as usize));
                self.total_bytes_written += written;
            }

            if res != flate2::Status::Ok {
                break;
            }
        }

        if fin {
            self.decode_continuation = false;
            if self.local_no_context_takeover {
                self.reset();
            }
        }

        Ok(output.into())
    }

    fn reset(&mut self) {
        self.decompress.reset(false);
        self.total_bytes_read = 0;
        self.total_bytes_written = 0;
    }
}

/// DEFLATE compression context.
#[derive(Debug)]
pub struct DeflateCompressionContext {
    pub(super) compression_level: flate2::Compression,
    pub(super) remote_no_context_takeover: bool,
    pub(super) remote_max_window_bits: u8,

    compress: flate2::Compress,
    total_bytes_written: u64,
    total_bytes_read: u64,
}

impl DeflateCompressionContext {
    pub(super) fn new(
        compression_level: Option<flate2::Compression>,
        remote_no_context_takeover: bool,
        remote_max_window_bits: u8,
    ) -> Self {
        let compression_level = compression_level.unwrap_or_default();

        Self {
            compression_level,
            remote_no_context_takeover,
            remote_max_window_bits,

            compress: flate2::Compress::new_with_window_bits(
                compression_level,
                false,
                remote_max_window_bits,
            ),

            total_bytes_written: 0,
            total_bytes_read: 0,
        }
    }

    pub(super) fn reset_with(
        mut self,
        remote_no_context_takeover: bool,
        remote_max_window_bits: u8,
    ) -> Self {
        self = Self::new(
            Some(self.compression_level),
            remote_no_context_takeover,
            remote_max_window_bits,
        );

        self
    }

    pub(super) fn compress(&mut self, fin: bool, payload: Bytes) -> Result<Bytes, ProtocolError> {
        let mut output = vec![];
        let mut buf = [0u8; BUF_SIZE];

        loop {
            let total_in = self.compress.total_in() - self.total_bytes_read;
            let res = if total_in >= payload.len() as u64 {
                self.compress
                    .compress(&[], &mut buf, flate2::FlushCompress::Sync)
                    .map_err(|err| {
                        self.reset();
                        ProtocolError::Io(err.into())
                    })?
            } else {
                self.compress
                    .compress(&payload, &mut buf, flate2::FlushCompress::None)
                    .map_err(|err| {
                        self.reset();
                        ProtocolError::Io(err.into())
                    })?
            };

            let written = self.compress.total_out() - self.total_bytes_written;
            if written > 0 {
                output.extend(buf.iter().take(written as usize));
                self.total_bytes_written += written;
            }

            if res != flate2::Status::Ok {
                break;
            }
        }
        self.total_bytes_read = self.compress.total_in();

        if output.iter().rev().take(4).eq(&[0xff, 0xff, 0x00, 0x00]) {
            output.drain(output.len() - 4..);
        }

        if fin && self.remote_no_context_takeover {
            self.reset();
        }

        Ok(output.into())
    }

    fn reset(&mut self) {
        self.compress.reset();
        self.total_bytes_read = 0;
        self.total_bytes_written = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::MessageBody;

    #[test]
    fn test_session_parameters() {
        let extension = "abc, def, permessage-deflate";
        assert_eq!(
            DeflateSessionParameters::from_extension_header(extension),
            vec![Ok(DeflateSessionParameters::default())]
        );

        let extension = "permessage-deflate; unknown_parameter";
        assert_eq!(
            DeflateSessionParameters::from_extension_header(extension),
            vec![Err(DeflateHandshakeError::UnknownWebSocketParameters)]
        );

        let extension = "permessage-deflate; client_max_window_bits=9; client_max_window_bits=10";
        assert_eq!(
            DeflateSessionParameters::from_extension_header(extension),
            vec![Err(DeflateHandshakeError::DuplicateParameter(
                "client_max_window_bits"
            ))]
        );

        let extension = "permessage-deflate; server_max_window_bits=8";
        assert_eq!(
            DeflateSessionParameters::from_extension_header(extension),
            vec![Err(DeflateHandshakeError::MaxWindowBitsOutOfRange)]
        );

        let extension = "permessage-deflate; server_max_window_bits=16";
        assert_eq!(
            DeflateSessionParameters::from_extension_header(extension),
            vec![Err(DeflateHandshakeError::MaxWindowBitsOutOfRange)]
        );

        let extension = "permessage-deflate; client_max_window_bits; server_max_window_bits=15; \
            client_no_context_takeover; server_no_context_takeover, \
            permessage-deflate; client_max_window_bits=10";
        assert_eq!(
            DeflateSessionParameters::from_extension_header(extension),
            vec![
                Ok(DeflateSessionParameters {
                    server_no_context_takeover: true,
                    client_no_context_takeover: true,
                    server_max_window_bits: Some(15),
                    client_max_window_bits: Some(ClientMaxWindowBits::NotSpecified)
                }),
                Ok(DeflateSessionParameters {
                    server_no_context_takeover: false,
                    client_no_context_takeover: false,
                    server_max_window_bits: None,
                    client_max_window_bits: Some(ClientMaxWindowBits::Specified(10))
                })
            ]
        );
    }

    #[test]
    fn test_compress() {
        // With context takeover

        let mut compress = DeflateCompressionContext::new(None, false, 15);
        assert_eq!(
            compress
                .compress(true, "Hello World".try_into_bytes().unwrap())
                .unwrap(),
            Bytes::from_static(b"\xf2H\xcd\xc9\xc9W\x08\xcf/\xcaI\x01\0")
        );
        assert_eq!(
            compress
                .compress(true, "Hello World".try_into_bytes().unwrap())
                .unwrap(),
            Bytes::from_static(b"\xf2@0\x01\0")
        );

        // Without context takeover

        let mut compress = DeflateCompressionContext::new(None, true, 15);
        assert_eq!(
            compress
                .compress(true, "Hello World".try_into_bytes().unwrap())
                .unwrap(),
            Bytes::from_static(b"\xf2H\xcd\xc9\xc9W\x08\xcf/\xcaI\x01\0")
        );
        assert_eq!(
            compress
                .compress(true, "Hello World".try_into_bytes().unwrap())
                .unwrap(),
            Bytes::from_static(b"\xf2H\xcd\xc9\xc9W\x08\xcf/\xcaI\x01\0")
        );

        // With continuation
        assert_eq!(
            compress
                .compress(false, "Hello World".try_into_bytes().unwrap())
                .unwrap(),
            Bytes::from_static(b"\xf2H\xcd\xc9\xc9W\x08\xcf/\xcaI\x01\0")
        );
        // Continuation keeps context.
        assert_eq!(
            compress
                .compress(true, "Hello World".try_into_bytes().unwrap())
                .unwrap(),
            Bytes::from_static(b"\xf2@0\x01\0")
        );
        // after continuation, context resets
        assert_eq!(
            compress
                .compress(true, "Hello World".try_into_bytes().unwrap())
                .unwrap(),
            Bytes::from_static(b"\xf2H\xcd\xc9\xc9W\x08\xcf/\xcaI\x01\0")
        );
    }

    #[test]
    fn test_decompress() {
        // With context takeover

        let mut decompress = DeflateDecompressionContext::new(false, 15);

        // Without RSV1 bit, decompression does not happen.
        assert_eq!(
            decompress
                .decompress(
                    true,
                    OpCode::Text,
                    RsvBits::empty(),
                    Bytes::from_static(b"Hello World")
                )
                .unwrap(),
            Bytes::from_static(b"Hello World")
        );

        // Control frames (such as ping/pong) are not decompressed
        assert_eq!(
            decompress
                .decompress(
                    true,
                    OpCode::Ping,
                    RsvBits::RSV1,
                    Bytes::from_static(b"Hello World")
                )
                .unwrap(),
            Bytes::from_static(b"Hello World")
        );

        // Successful decompression
        assert_eq!(
            decompress
                .decompress(
                    true,
                    OpCode::Text,
                    RsvBits::RSV1,
                    Bytes::from_static(b"\xf2H\xcd\xc9\xc9W\x08\xcf/\xcaI\x01\0")
                )
                .unwrap(),
            Bytes::from_static(b"Hello World")
        );

        // Success subsequent decompression
        assert_eq!(
            decompress
                .decompress(
                    true,
                    OpCode::Text,
                    RsvBits::RSV1,
                    Bytes::from_static(b"\xf2@0\x01\0")
                )
                .unwrap(),
            Bytes::from_static(b"Hello World")
        );

        // Invalid compression payload
        assert!(decompress
            .decompress(
                true,
                OpCode::Text,
                RsvBits::RSV1,
                Bytes::from_static(b"Hello World")
            )
            .is_err());

        // When there was error, context is reset.
        assert_eq!(
            decompress
                .decompress(
                    true,
                    OpCode::Text,
                    RsvBits::RSV1,
                    Bytes::from_static(b"\xf2H\xcd\xc9\xc9W\x08\xcf/\xcaI\x01\0")
                )
                .unwrap(),
            Bytes::from_static(b"Hello World")
        );

        // Without context takeover

        let mut decompress = DeflateDecompressionContext::new(true, 15);

        // Successful decompression
        assert_eq!(
            decompress
                .decompress(
                    true,
                    OpCode::Text,
                    RsvBits::RSV1,
                    Bytes::from_static(b"\xf2H\xcd\xc9\xc9W\x08\xcf/\xcaI\x01\0")
                )
                .unwrap(),
            Bytes::from_static(b"Hello World")
        );

        // Context has been reset.
        assert_eq!(
            decompress
                .decompress(
                    true,
                    OpCode::Text,
                    RsvBits::RSV1,
                    Bytes::from_static(b"\xf2H\xcd\xc9\xc9W\x08\xcf/\xcaI\x01\0")
                )
                .unwrap(),
            Bytes::from_static(b"Hello World")
        );

        // With continuation
        assert_eq!(
            decompress
                .decompress(
                    false,
                    OpCode::Text,
                    RsvBits::RSV1,
                    Bytes::from_static(b"\xf2H\xcd\xc9\xc9W\x08\xcf/\xcaI\x01\0")
                )
                .unwrap(),
            Bytes::from_static(b"Hello World")
        );
        // Continuation keeps context.
        assert_eq!(
            decompress
                .decompress(
                    true,
                    OpCode::Text,
                    RsvBits::RSV1,
                    Bytes::from_static(b"\xf2@0\x01\0")
                )
                .unwrap(),
            Bytes::from_static(b"Hello World")
        );
        // When continuation has finished, context is reset.
        assert_eq!(
            decompress
                .decompress(
                    false,
                    OpCode::Text,
                    RsvBits::RSV1,
                    Bytes::from_static(b"\xf2H\xcd\xc9\xc9W\x08\xcf/\xcaI\x01\0")
                )
                .unwrap(),
            Bytes::from_static(b"Hello World")
        );
    }
}
