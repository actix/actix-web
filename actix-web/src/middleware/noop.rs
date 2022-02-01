//! A no-op middleware. See [Noop] for docs.

use actix_utils::future::{ready, Ready};

use crate::dev::{Service, Transform};

/// A no-op middleware that passes through request and response untouched.
pub(crate) struct Noop;

impl<S: Service<Req>, Req> Transform<S, Req> for Noop {
    type Response = S::Response;
    type Error = S::Error;
    type Transform = NoopService<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(NoopService { service }))
    }
}

#[doc(hidden)]
pub(crate) struct NoopService<S> {
    service: S,
}

impl<S: Service<Req>, Req> Service<Req> for NoopService<S> {
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    crate::dev::forward_ready!(service);

    fn call(&self, req: Req) -> Self::Future {
        self.service.call(req)
    }
}
