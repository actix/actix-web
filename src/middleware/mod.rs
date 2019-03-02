use std::marker::PhantomData;

use actix_service::{NewTransform, Service, Transform};
use futures::future::{ok, FutureResult};

#[cfg(any(feature = "brotli", feature = "flate2"))]
mod compress;
#[cfg(any(feature = "brotli", feature = "flate2"))]
pub use self::compress::Compress;

mod defaultheaders;
pub use self::defaultheaders::DefaultHeaders;

/// Helper for middleware service factory
pub struct MiddlewareFactory<T, S>
where
    T: Transform<S> + Clone,
    S: Service,
{
    tr: T,
    _t: PhantomData<S>,
}

impl<T, S> MiddlewareFactory<T, S>
where
    T: Transform<S> + Clone,
    S: Service,
{
    pub fn new(tr: T) -> Self {
        MiddlewareFactory {
            tr,
            _t: PhantomData,
        }
    }
}

impl<T, S> Clone for MiddlewareFactory<T, S>
where
    T: Transform<S> + Clone,
    S: Service,
{
    fn clone(&self) -> Self {
        Self {
            tr: self.tr.clone(),
            _t: PhantomData,
        }
    }
}

impl<T, S, C> NewTransform<S, C> for MiddlewareFactory<T, S>
where
    T: Transform<S> + Clone,
    S: Service,
{
    type Request = T::Request;
    type Response = T::Response;
    type Error = T::Error;
    type Transform = T;
    type InitError = ();
    type Future = FutureResult<Self::Transform, Self::InitError>;

    fn new_transform(&self, _: &C) -> Self::Future {
        ok(self.tr.clone())
    }
}
