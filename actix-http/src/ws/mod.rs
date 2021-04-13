//! WebSocket protocol implementation.
//!
//! To setup a WebSocket, first perform the WebSocket handshake then on success convert `Payload` into a
//! `WsStream` stream and then use `WsWriter` to communicate with the peer.

use std::io;

use derive_more::{Display, Error, From};
use http::{header, Method, StatusCode};

use crate::{
    body::Body, error::ResponseError, header::HeaderValue, message::RequestHead,
    response::Response, ResponseBuilder,
};

mod codec;
mod dispatcher;
mod frame;
mod mask;
mod proto;

pub use self::codec::{Codec, Frame, Item, Message};
pub use self::dispatcher::Dispatcher;
pub use self::frame::Parser;
pub use self::proto::{hash_key, CloseCode, CloseReason, OpCode};

/// WebSocket protocol errors.
#[derive(Debug, Display, From, Error)]
pub enum ProtocolError {
    /// Received an unmasked frame from client.
    #[display(fmt = "Received an unmasked frame from client.")]
    UnmaskedFrame,

    /// Received a masked frame from server.
    #[display(fmt = "Received a masked frame from server.")]
    MaskedFrame,

    /// Encountered invalid opcode.
    #[display(fmt = "Invalid opcode: {}.", _0)]
    InvalidOpcode(#[error(not(source))] u8),

    /// Invalid control frame length
    #[display(fmt = "Invalid control frame length: {}.", _0)]
    InvalidLength(#[error(not(source))] usize),

    /// Bad opcode.
    #[display(fmt = "Bad opcode.")]
    BadOpCode,

    /// A payload reached size limit.
    #[display(fmt = "A payload reached size limit.")]
    Overflow,

    /// Continuation is not started.
    #[display(fmt = "Continuation is not started.")]
    ContinuationNotStarted,

    /// Received new continuation but it is already started.
    #[display(fmt = "Received new continuation but it is already started.")]
    ContinuationStarted,

    /// Unknown continuation fragment.
    #[display(fmt = "Unknown continuation fragment: {}.", _0)]
    ContinuationFragment(#[error(not(source))] OpCode),

    /// I/O error.
    #[display(fmt = "I/O error: {}", _0)]
    Io(io::Error),
}

impl ResponseError for ProtocolError {}

/// WebSocket handshake errors
#[derive(PartialEq, Debug, Display)]
pub enum HandshakeError {
    /// Only get method is allowed.
    #[display(fmt = "Method not allowed.")]
    GetMethodRequired,

    /// Upgrade header if not set to WebSocket.
    #[display(fmt = "WebSocket upgrade is expected.")]
    NoWebsocketUpgrade,

    /// Connection header is not set to upgrade.
    #[display(fmt = "Connection upgrade is expected.")]
    NoConnectionUpgrade,

    /// WebSocket version header is not set.
    #[display(fmt = "WebSocket version header is required.")]
    NoVersionHeader,

    /// Unsupported WebSocket version.
    #[display(fmt = "Unsupported WebSocket version.")]
    UnsupportedVersion,

    /// WebSocket key is not set or wrong.
    #[display(fmt = "Unknown websocket key.")]
    BadWebsocketKey,
}

impl ResponseError for HandshakeError {
    fn error_response(&self) -> Response<Body> {
        match self {
            HandshakeError::GetMethodRequired => Response::MethodNotAllowed()
                .insert_header((header::ALLOW, "GET"))
                .finish(),

            HandshakeError::NoWebsocketUpgrade => Response::BadRequest()
                .reason("No WebSocket Upgrade header found")
                .finish(),

            HandshakeError::NoConnectionUpgrade => Response::BadRequest()
                .reason("No Connection upgrade")
                .finish(),

            HandshakeError::NoVersionHeader => Response::BadRequest()
                .reason("WebSocket version header is required")
                .finish(),

            HandshakeError::UnsupportedVersion => Response::BadRequest()
                .reason("Unsupported WebSocket version")
                .finish(),

            HandshakeError::BadWebsocketKey => {
                Response::BadRequest().reason("Handshake error").finish()
            }
        }
    }
}

/// Verify WebSocket handshake request and create handshake response.
pub fn handshake(req: &RequestHead) -> Result<ResponseBuilder, HandshakeError> {
    verify_handshake(req)?;
    Ok(handshake_response(req))
}

/// Verify WebSocket handshake request.
pub fn verify_handshake(req: &RequestHead) -> Result<(), HandshakeError> {
    // WebSocket accepts only GET
    if req.method != Method::GET {
        return Err(HandshakeError::GetMethodRequired);
    }

    // Check for "UPGRADE" to WebSocket header
    let has_hdr = if let Some(hdr) = req.headers().get(header::UPGRADE) {
        if let Ok(s) = hdr.to_str() {
            s.to_ascii_lowercase().contains("websocket")
        } else {
            false
        }
    } else {
        false
    };
    if !has_hdr {
        return Err(HandshakeError::NoWebsocketUpgrade);
    }

    // Upgrade connection
    if !req.upgrade() {
        return Err(HandshakeError::NoConnectionUpgrade);
    }

    // check supported version
    if !req.headers().contains_key(header::SEC_WEBSOCKET_VERSION) {
        return Err(HandshakeError::NoVersionHeader);
    }
    let supported_ver = {
        if let Some(hdr) = req.headers().get(header::SEC_WEBSOCKET_VERSION) {
            hdr == "13" || hdr == "8" || hdr == "7"
        } else {
            false
        }
    };
    if !supported_ver {
        return Err(HandshakeError::UnsupportedVersion);
    }

    // check client handshake for validity
    if !req.headers().contains_key(header::SEC_WEBSOCKET_KEY) {
        return Err(HandshakeError::BadWebsocketKey);
    }
    Ok(())
}

/// Create WebSocket handshake response.
///
/// This function returns handshake `Response`, ready to send to peer.
pub fn handshake_response(req: &RequestHead) -> ResponseBuilder {
    let key = {
        let key = req.headers().get(header::SEC_WEBSOCKET_KEY).unwrap();
        proto::hash_key(key.as_ref())
    };

    Response::build(StatusCode::SWITCHING_PROTOCOLS)
        .upgrade("websocket")
        .insert_header((header::TRANSFER_ENCODING, "chunked"))
        .insert_header((
            header::SEC_WEBSOCKET_ACCEPT,
            // key is known to be header value safe ascii
            HeaderValue::from_bytes(&key).unwrap(),
        ))
        .take()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test::TestRequest;
    use http::{header, Method};

    #[test]
    fn test_handshake() {
        let req = TestRequest::default().method(Method::POST).finish();
        assert_eq!(
            HandshakeError::GetMethodRequired,
            verify_handshake(req.head()).unwrap_err(),
        );

        let req = TestRequest::default().finish();
        assert_eq!(
            HandshakeError::NoWebsocketUpgrade,
            verify_handshake(req.head()).unwrap_err(),
        );

        let req = TestRequest::default()
            .insert_header((header::UPGRADE, header::HeaderValue::from_static("test")))
            .finish();
        assert_eq!(
            HandshakeError::NoWebsocketUpgrade,
            verify_handshake(req.head()).unwrap_err(),
        );

        let req = TestRequest::default()
            .insert_header((
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            ))
            .finish();
        assert_eq!(
            HandshakeError::NoConnectionUpgrade,
            verify_handshake(req.head()).unwrap_err(),
        );

        let req = TestRequest::default()
            .insert_header((
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            ))
            .insert_header((
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            ))
            .finish();
        assert_eq!(
            HandshakeError::NoVersionHeader,
            verify_handshake(req.head()).unwrap_err(),
        );

        let req = TestRequest::default()
            .insert_header((
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            ))
            .insert_header((
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            ))
            .insert_header((
                header::SEC_WEBSOCKET_VERSION,
                header::HeaderValue::from_static("5"),
            ))
            .finish();
        assert_eq!(
            HandshakeError::UnsupportedVersion,
            verify_handshake(req.head()).unwrap_err(),
        );

        let req = TestRequest::default()
            .insert_header((
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            ))
            .insert_header((
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            ))
            .insert_header((
                header::SEC_WEBSOCKET_VERSION,
                header::HeaderValue::from_static("13"),
            ))
            .finish();
        assert_eq!(
            HandshakeError::BadWebsocketKey,
            verify_handshake(req.head()).unwrap_err(),
        );

        let req = TestRequest::default()
            .insert_header((
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            ))
            .insert_header((
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            ))
            .insert_header((
                header::SEC_WEBSOCKET_VERSION,
                header::HeaderValue::from_static("13"),
            ))
            .insert_header((
                header::SEC_WEBSOCKET_KEY,
                header::HeaderValue::from_static("13"),
            ))
            .finish();
        assert_eq!(
            StatusCode::SWITCHING_PROTOCOLS,
            handshake_response(req.head()).finish().status()
        );
    }

    #[test]
    fn test_wserror_http_response() {
        let resp = HandshakeError::GetMethodRequired.error_response();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
        let resp = HandshakeError::NoWebsocketUpgrade.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let resp = HandshakeError::NoConnectionUpgrade.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let resp = HandshakeError::NoVersionHeader.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let resp = HandshakeError::UnsupportedVersion.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let resp = HandshakeError::BadWebsocketKey.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
