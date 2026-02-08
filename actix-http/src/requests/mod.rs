//! HTTP requests.

mod head;
mod request;

pub use self::{
    head::{RequestHead, RequestHeadType},
    request::Request,
};
