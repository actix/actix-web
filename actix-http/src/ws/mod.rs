//! WebSocket protocol implementation.
//!
//! To setup a WebSocket, first perform the WebSocket handshake then on success convert `Payload` into a
//! `WsStream` stream and then use `WsWriter` to communicate with the peer.

use std::io;

use derive_more::{Display, Error, From};
use http::{header, Method, StatusCode};

use crate::{body::BoxBody, header::HeaderValue, RequestHead, Response, ResponseBuilder};

mod codec;
mod dispatcher;
mod frame;
mod mask;
mod proto;

pub use self::{
    codec::{Codec, Frame, Item, Message},
    dispatcher::Dispatcher,
    frame::Parser,
    proto::{hash_key, CloseCode, CloseReason, OpCode},
};

/// WebSocket protocol errors.
#[derive(Debug, Display, Error, From)]
pub enum ProtocolError {
    /// Received an unmasked frame from client.
    #[display(fmt = "received an unmasked frame from client")]
    UnmaskedFrame,

    /// Received a masked frame from server.
    #[display(fmt = "received a masked frame from server")]
    MaskedFrame,

    /// Encountered invalid opcode.
    #[display(fmt = "invalid opcode ({})", _0)]
    InvalidOpcode(#[error(not(source))] u8),

    /// Invalid control frame length
    #[display(fmt = "invalid control frame length ({})", _0)]
    InvalidLength(#[error(not(source))] usize),

    /// Bad opcode.
    #[display(fmt = "bad opcode")]
    BadOpCode,

    /// A payload reached size limit.
    #[display(fmt = "payload reached size limit")]
    Overflow,

    /// Continuation has not started.
    #[display(fmt = "continuation has not started")]
    ContinuationNotStarted,

    /// Received new continuation but it is already started.
    #[display(fmt = "received new continuation but it has already started")]
    ContinuationStarted,

    /// Unknown continuation fragment.
    #[display(fmt = "unknown continuation fragment: {}", _0)]
    ContinuationFragment(#[error(not(source))] OpCode),

    /// I/O error.
    #[display(fmt = "I/O error: {}", _0)]
    Io(io::Error),
}

/// WebSocket handshake errors
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, Error)]
pub enum HandshakeError {
    /// Only get method is allowed.
    #[display(fmt = "method not allowed")]
    GetMethodRequired,

    /// Upgrade header if not set to WebSocket.
    #[display(fmt = "WebSocket upgrade is expected")]
    NoWebsocketUpgrade,

    /// Connection header is not set to upgrade.
    #[display(fmt = "connection upgrade is expected")]
    NoConnectionUpgrade,

    /// WebSocket version header is not set.
    #[display(fmt = "WebSocket version header is required")]
    NoVersionHeader,

    /// Unsupported WebSocket version.
    #[display(fmt = "unsupported WebSocket version")]
    UnsupportedVersion,

    /// WebSocket key is not set or wrong.
    #[display(fmt = "unknown WebSocket key")]
    BadWebsocketKey,
}

impl From<HandshakeError> for Response<BoxBody> {
    fn from(err: HandshakeError) -> Self {
        match err {
            HandshakeError::GetMethodRequired => {
                let mut res = Response::new(StatusCode::METHOD_NOT_ALLOWED);
                #[allow(clippy::declare_interior_mutable_const)]
                const HV_GET: HeaderValue = HeaderValue::from_static("GET");
                res.headers_mut().insert(header::ALLOW, HV_GET);
                res
            }

            HandshakeError::NoWebsocketUpgrade => {
                let mut res = Response::bad_request();
                res.head_mut().reason = Some("No WebSocket Upgrade header found");
                res
            }

            HandshakeError::NoConnectionUpgrade => {
                let mut res = Response::bad_request();
                res.head_mut().reason = Some("No Connection upgrade");
                res
            }

            HandshakeError::NoVersionHeader => {
                let mut res = Response::bad_request();
                res.head_mut().reason = Some("WebSocket version header is required");
                res
            }

            HandshakeError::UnsupportedVersion => {
                let mut res = Response::bad_request();
                res.head_mut().reason = Some("Unsupported WebSocket version");
                res
            }

            HandshakeError::BadWebsocketKey => {
                let mut res = Response::bad_request();
                res.head_mut().reason = Some("Handshake error");
                res
            }
        }
    }
}

impl From<&HandshakeError> for Response<BoxBody> {
    fn from(err: &HandshakeError) -> Self {
        (*err).into()
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
    use crate::{header, test::TestRequest};

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
    fn test_ws_error_http_response() {
        let resp: Response<BoxBody> = HandshakeError::GetMethodRequired.into();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
        let resp: Response<BoxBody> = HandshakeError::NoWebsocketUpgrade.into();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let resp: Response<BoxBody> = HandshakeError::NoConnectionUpgrade.into();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let resp: Response<BoxBody> = HandshakeError::NoVersionHeader.into();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let resp: Response<BoxBody> = HandshakeError::UnsupportedVersion.into();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let resp: Response<BoxBody> = HandshakeError::BadWebsocketKey.into();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
