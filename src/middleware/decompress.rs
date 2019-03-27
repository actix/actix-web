//! Chain service for decompressing request payload.
use std::marker::PhantomData;

use actix_http::encoding::Decoder;
use actix_service::{NewService, Service};
use bytes::Bytes;
use futures::future::{ok, FutureResult};
use futures::{Async, Poll, Stream};

use crate::dev::Payload;
use crate::error::{Error, PayloadError};
use crate::service::ServiceRequest;
use crate::HttpMessage;

/// `Middleware` for decompressing request's payload.
/// `Decompress` middleware must be added with `App::chain()` method.
///
/// ```rust
/// use actix_web::{web, middleware::encoding, App, HttpResponse};
///
/// fn main() {
///     let app = App::new()
///         .chain(encoding::Decompress::new())
///         .service(
///             web::resource("/test")
///                 .route(web::get().to(|| HttpResponse::Ok()))
///                 .route(web::head().to(|| HttpResponse::MethodNotAllowed()))
///         );
/// }
/// ```
pub struct Decompress<P>(PhantomData<P>);

impl<P> Decompress<P>
where
    P: Stream<Item = Bytes, Error = PayloadError>,
{
    pub fn new() -> Self {
        Decompress(PhantomData)
    }
}

impl<P> NewService for Decompress<P>
where
    P: Stream<Item = Bytes, Error = PayloadError>,
{
    type Request = ServiceRequest<P>;
    type Response = ServiceRequest<Decoder<Payload<P>>>;
    type Error = Error;
    type InitError = ();
    type Service = Decompress<P>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self, _: &()) -> Self::Future {
        ok(Decompress(PhantomData))
    }
}

impl<P> Service for Decompress<P>
where
    P: Stream<Item = Bytes, Error = PayloadError>,
{
    type Request = ServiceRequest<P>;
    type Response = ServiceRequest<Decoder<Payload<P>>>;
    type Error = Error;
    type Future = FutureResult<Self::Response, Self::Error>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: ServiceRequest<P>) -> Self::Future {
        let (req, payload) = req.into_parts();
        let payload = Decoder::from_headers(req.headers(), payload);
        ok(ServiceRequest::from_parts(req, Payload::Stream(payload)))
    }
}
