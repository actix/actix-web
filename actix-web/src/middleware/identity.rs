//! A no-op middleware. See [Noop] for docs.

use actix_utils::future::{ready, Ready};

use crate::dev::{forward_ready, Service, Transform};

/// A no-op middleware that passes through request and response untouched.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Identity;

impl<S: Service<Req>, Req> Transform<S, Req> for Identity {
    type Response = S::Response;
    type Error = S::Error;
    type Transform = IdentityMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    #[inline]
    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(IdentityMiddleware { service }))
    }
}

#[doc(hidden)]
pub struct IdentityMiddleware<S> {
    service: S,
}

impl<S: Service<Req>, Req> Service<Req> for IdentityMiddleware<S> {
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    forward_ready!(service);

    #[inline]
    fn call(&self, req: Req) -> Self::Future {
        self.service.call(req)
    }
}
