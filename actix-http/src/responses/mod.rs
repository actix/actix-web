//! HTTP response.

mod builder;
mod head;
#[allow(clippy::module_inception)]
mod response;

pub(crate) use self::head::BoxedResponseHead;
pub use self::{builder::ResponseBuilder, head::ResponseHead, response::Response};
