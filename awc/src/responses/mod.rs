use std::{future::Future, io, pin::Pin, task::Context};

use actix_http::error::PayloadError;
use actix_rt::time::Sleep;

mod json_body;
mod read_body;
mod response;
mod response_body;

#[allow(deprecated)]
pub use self::response_body::{MessageBody, ResponseBody};
pub use self::{json_body::JsonBody, response::ClientResponse};

/// Default body size limit: 2 MiB
const DEFAULT_BODY_LIMIT: usize = 2 * 1024 * 1024;

/// Builds a response whose payload is decompressed like one from `SendClientRequest`, with `data`
/// gzip-compressed into a single chunk.
#[cfg(all(test, feature = "compress-gzip"))]
fn gzip_response(
    data: &[u8],
) -> ClientResponse<actix_http::encoding::Decoder<actix_http::Payload>> {
    use std::io::Write as _;

    use actix_http::{
        encoding::Decoder, header::ContentEncoding, BoxedPayloadStream, Payload, ResponseHead,
        StatusCode,
    };
    use bytes::Bytes;

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(data).unwrap();
    let chunk = Bytes::from(encoder.finish().unwrap());

    let stream: BoxedPayloadStream = Box::pin(futures_util::stream::once(async { Ok(chunk) }));
    let payload = Payload::Stream {
        payload: Decoder::new(Payload::from(stream), ContentEncoding::Gzip),
    };

    let mut head = ResponseHead::new(StatusCode::OK);
    head.headers.insert(
        actix_http::header::CONTENT_TYPE,
        actix_http::header::HeaderValue::from_static("application/json"),
    );

    ClientResponse::new(head, payload)._decode_limit(crate::sender::limit_decoded_payload)
}

/// Helper enum with reusable sleep passed from `SendClientResponse`.
///
/// See [`ClientResponse::_timeout`] for reason.
pub(crate) enum ResponseTimeout {
    Disabled(Option<Pin<Box<Sleep>>>),
    Enabled(Pin<Box<Sleep>>),
}

impl Default for ResponseTimeout {
    fn default() -> Self {
        Self::Disabled(None)
    }
}

impl ResponseTimeout {
    fn poll_timeout(&mut self, cx: &mut Context<'_>) -> Result<(), PayloadError> {
        match *self {
            Self::Enabled(ref mut timeout) => {
                if timeout.as_mut().poll(cx).is_ready() {
                    Err(PayloadError::Io(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "Response Payload IO timed out",
                    )))
                } else {
                    Ok(())
                }
            }
            Self::Disabled(_) => Ok(()),
        }
    }
}
