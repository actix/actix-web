use std::ops::{Deref, DerefMut};

use actix_http::error::{Error, ErrorInternalServerError};
use futures_util::future;

use crate::{dev::Payload, FromRequest, HttpRequest};

/// Request data.
///
/// Request data is a piece of arbitrary data attached to a request.
///
/// It can be set via [`HttpMessage::extensions_mut`].
///
/// [`HttpMessage::extensions_mut`]: crate::HttpMessage::extensions_mut
#[derive(Clone, Debug)]
pub struct ReqData<T: Clone + 'static>(pub T);

impl<T: Clone + 'static> Deref for ReqData<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: Clone + 'static> DerefMut for ReqData<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: Clone + 'static> FromRequest for ReqData<T> {
    type Config = ();
    type Error = Error;
    type Future = future::Ready<Result<Self, Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        if let Some(st) = req.extensions().get::<T>() {
            future::ok(ReqData(st.clone()))
        } else {
            log::debug!(
                "Failed to construct App-level ReqData extractor. \
                 Request path: {:?}",
                req.path()
            );
            future::err(ErrorInternalServerError(
                "Missing expected request extension data",
            ))
        }
    }
}
