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
