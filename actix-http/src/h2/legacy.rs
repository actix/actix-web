//! Compatibility with the public h2 0.3 error types.

use std::{
    future::Future,
    io,
    pin::{pin, Pin},
    task::{Context, Poll, Waker},
};

use bytes::Bytes;
use h2::proto::{Error as ProtocolError, Initiator};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// Preserve the legacy error type without exposing the current backend.
///
/// The backend does not expose reset stream IDs or GOAWAY debug bytes. A reset
/// therefore uses stream ID zero, and a GOAWAY omits its debug bytes. Classification,
/// reason codes, and origin are retained. This changes some formatted diagnostics.
///
/// User errors have no structured accessor. Known messages are matched against
/// the old library; unknown messages become I/O errors with kind `Other`.
pub(super) fn error(err: h2_04::Error) -> h2::Error {
    if err.is_io() {
        return io_error(err.into_io().expect("is_io checked above"));
    }

    if let Some(reason) = err.reason() {
        let reason = h2::Reason::from(u32::from(reason));
        let initiator = if err.is_remote() {
            Initiator::Remote
        } else if err.is_library() {
            Initiator::Library
        } else {
            Initiator::User
        };

        return if err.is_reset() {
            ProtocolError::Reset(h2::frame::StreamId::zero(), reason, initiator).into()
        } else if err.is_go_away() {
            ProtocolError::GoAway(Bytes::new(), reason, initiator).into()
        } else {
            reason.into()
        };
    }

    user_error(err.to_string())
}

fn user_error(message: String) -> h2::Error {
    use h2::UserError::*;

    let known = [
        InactiveStreamId,
        UnexpectedFrameType,
        PayloadTooBig,
        Rejected,
        ReleaseCapacityTooBig,
        OverflowedStreamId,
        MalformedHeaders,
        MissingUriSchemeAndAuthority,
        PollResetAfterSendResponse,
        SendPingWhilePending,
        SendSettingsWhilePending,
        PeerDisabledServerPush,
    ];

    match known
        .into_iter()
        .map(h2::Error::from)
        .find(|err| err.to_string() == message)
    {
        Some(err) => err,
        None => io_error(io::Error::other(message)),
    }
}

/// The legacy I/O constructor is private. Its client handshake wraps the first
/// write error directly, so this preserves the original OS code and inner error.
fn io_error(err: io::Error) -> h2::Error {
    let mut handshake = pin!(h2::client::handshake(FailWrite(Some(err))));
    let mut cx = Context::from_waker(Waker::noop());

    match handshake.as_mut().poll(&mut cx) {
        Poll::Ready(Err(err)) => err,
        // FailWrite always fails the first preface write, without a socket or runtime.
        _ => unreachable!("legacy handshake must return the first write error"),
    }
}

struct FailWrite(Option<io::Error>);

impl AsyncRead for FailWrite {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        unreachable!("legacy handshake writes the preface before reading")
    }
}

impl AsyncWrite for FailWrite {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(Err(self.0.take().expect("only one write is attempted")))
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        unreachable!("legacy handshake stops at the first write error")
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        unreachable!("legacy handshake stops at the first write error")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_errors_retain_reason_classification_and_origin() {
        for initiator in [
            h2_04::proto::Initiator::User,
            h2_04::proto::Initiator::Library,
            h2_04::proto::Initiator::Remote,
        ] {
            for reset in [false, true] {
                let reason = h2_04::Reason::from(0xf00);
                let original = h2_04::Error::from(if reset {
                    h2_04::proto::Error::Reset(123.into(), reason, initiator)
                } else {
                    h2_04::proto::Error::GoAway(Bytes::from_static(b"detail"), reason, initiator)
                });
                let expected_remote = initiator == h2_04::proto::Initiator::Remote;
                let expected_library = initiator == h2_04::proto::Initiator::Library;

                let converted = error(original);

                assert_eq!(converted.reason().map(u32::from), Some(0xf00));
                assert_eq!(converted.is_reset(), reset);
                assert_eq!(converted.is_go_away(), !reset);
                assert_eq!(converted.is_remote(), expected_remote);
                assert_eq!(converted.is_library(), expected_library);
                assert!(!converted.is_io());
            }
        }
    }

    #[test]
    fn bare_reason_does_not_become_a_reset() {
        let converted = error(h2_04::Reason::CANCEL.into());

        assert_eq!(converted.reason(), Some(h2::Reason::CANCEL));
        assert!(!converted.is_reset());
        assert!(!converted.is_go_away());
        assert!(!converted.is_remote());
        assert!(!converted.is_library());
    }

    fn backend_io(err: io::Error) -> h2_04::Error {
        let mut handshake = pin!(h2_04::client::handshake(FailWrite(Some(err))));
        let mut cx = Context::from_waker(Waker::noop());

        match handshake.as_mut().poll(&mut cx) {
            Poll::Ready(Err(err)) => err,
            _ => panic!("expected first write to fail"),
        }
    }

    #[test]
    fn io_error_retains_os_code() {
        let converted = error(backend_io(io::Error::from_raw_os_error(13)));

        assert!(converted.is_io());
        assert_eq!(converted.into_io().unwrap().raw_os_error(), Some(13));
    }

    #[test]
    fn io_error_retains_inner_error() {
        #[derive(Debug)]
        struct Marker;

        impl std::fmt::Display for Marker {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("original transport failure")
            }
        }

        impl std::error::Error for Marker {}

        let converted = error(backend_io(io::Error::other(Marker)));
        let inner = converted.get_io().unwrap().get_ref().unwrap();

        assert!(inner.is::<Marker>());
        assert_eq!(converted.to_string(), "original transport failure");
    }

    #[test]
    fn known_user_error_retains_legacy_classification() {
        let converted = error(h2_04::UserError::ReleaseCapacityTooBig.into());

        assert_eq!(
            converted.to_string(),
            "user error: release capacity too big"
        );
        assert!(converted.reason().is_none());
        assert!(!converted.is_io());
        assert!(!converted.is_reset());
        assert!(!converted.is_go_away());
    }

    #[test]
    fn unknown_user_error_retains_message_without_exposing_backend() {
        let original = h2_04::Error::from(h2_04::UserError::InvalidInformationalStatusCode);
        let message = original.to_string();
        let converted = error(original);

        assert!(converted.is_io());
        assert_eq!(converted.get_io().unwrap().kind(), io::ErrorKind::Other);
        assert_eq!(converted.to_string(), message);
        assert!(!converted
            .get_io()
            .unwrap()
            .get_ref()
            .unwrap()
            .is::<h2_04::Error>());
    }
}
