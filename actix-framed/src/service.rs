use std::marker::PhantomData;

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_http::error::{Error, ResponseError};
use actix_http::ws::{verify_handshake, HandshakeError};
use actix_http::{h1, Request};
use actix_service::{NewService, Service};
use futures::future::{ok, Either, FutureResult};
use futures::{Async, Future, IntoFuture, Poll};

/// Service that verifies incoming request if it is valid websocket
/// upgrade request. In case of error returns `HandshakeError`
pub struct VerifyWebSockets<T> {
    _t: PhantomData<T>,
}

impl<T> Default for VerifyWebSockets<T> {
    fn default() -> Self {
        VerifyWebSockets { _t: PhantomData }
    }
}

impl<T> NewService for VerifyWebSockets<T> {
    type Request = (Request, Framed<T, h1::Codec>);
    type Response = (Request, Framed<T, h1::Codec>);
    type Error = (HandshakeError, Framed<T, h1::Codec>);
    type InitError = ();
    type Service = VerifyWebSockets<T>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self, _: &()) -> Self::Future {
        ok(VerifyWebSockets { _t: PhantomData })
    }
}

impl<T> Service for VerifyWebSockets<T> {
    type Request = (Request, Framed<T, h1::Codec>);
    type Response = (Request, Framed<T, h1::Codec>);
    type Error = (HandshakeError, Framed<T, h1::Codec>);
    type Future = FutureResult<Self::Response, Self::Error>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, (req, framed): (Request, Framed<T, h1::Codec>)) -> Self::Future {
        match verify_handshake(req.head()) {
            Err(e) => Err((e, framed)).into_future(),
            Ok(_) => Ok((req, framed)).into_future(),
        }
    }
}

/// Send http/1 error response
pub struct SendError<T, R, E>(PhantomData<(T, R, E)>);

impl<T, R, E> Default for SendError<T, R, E>
where
    T: AsyncRead + AsyncWrite,
    E: ResponseError,
{
    fn default() -> Self {
        SendError(PhantomData)
    }
}

impl<T, R, E> NewService for SendError<T, R, E>
where
    T: AsyncRead + AsyncWrite + 'static,
    R: 'static,
    E: ResponseError + 'static,
{
    type Request = Result<R, (E, Framed<T, h1::Codec>)>;
    type Response = R;
    type Error = Error;
    type InitError = ();
    type Service = SendError<T, R, E>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self, _: &()) -> Self::Future {
        ok(SendError(PhantomData))
    }
}

impl<T, R, E> Service for SendError<T, R, E>
where
    T: AsyncRead + AsyncWrite + 'static,
    R: 'static,
    E: ResponseError + 'static,
{
    type Request = Result<R, (E, Framed<T, h1::Codec>)>;
    type Response = R;
    type Error = Error;
    type Future = Either<FutureResult<R, Error>, Box<Future<Item = R, Error = Error>>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Result<R, (E, Framed<T, h1::Codec>)>) -> Self::Future {
        match req {
            Ok(r) => Either::A(ok(r)),
            Err((e, framed)) => {
                let res = e.render_response();
                let e = Error::from(e);
                Either::B(Box::new(
                    h1::SendResponse::new(framed, res).then(move |_| Err(e)),
                ))
            }
        }
    }
}
