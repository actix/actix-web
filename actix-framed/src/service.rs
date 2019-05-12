use std::marker::PhantomData;

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_http::body::BodySize;
use actix_http::error::ResponseError;
use actix_http::h1::{Codec, Message};
use actix_http::ws::{verify_handshake, HandshakeError};
use actix_http::{Request, Response};
use actix_service::{NewService, Service};
use futures::future::{ok, Either, FutureResult};
use futures::{Async, Future, IntoFuture, Poll, Sink};

/// Service that verifies incoming request if it is valid websocket
/// upgrade request. In case of error returns `HandshakeError`
pub struct VerifyWebSockets<T, C> {
    _t: PhantomData<(T, C)>,
}

impl<T, C> Default for VerifyWebSockets<T, C> {
    fn default() -> Self {
        VerifyWebSockets { _t: PhantomData }
    }
}

impl<T, C> NewService for VerifyWebSockets<T, C> {
    type Config = C;
    type Request = (Request, Framed<T, Codec>);
    type Response = (Request, Framed<T, Codec>);
    type Error = (HandshakeError, Framed<T, Codec>);
    type InitError = ();
    type Service = VerifyWebSockets<T, C>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self, _: &C) -> Self::Future {
        ok(VerifyWebSockets { _t: PhantomData })
    }
}

impl<T, C> Service for VerifyWebSockets<T, C> {
    type Request = (Request, Framed<T, Codec>);
    type Response = (Request, Framed<T, Codec>);
    type Error = (HandshakeError, Framed<T, Codec>);
    type Future = FutureResult<Self::Response, Self::Error>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, (req, framed): (Request, Framed<T, Codec>)) -> Self::Future {
        match verify_handshake(req.head()) {
            Err(e) => Err((e, framed)).into_future(),
            Ok(_) => Ok((req, framed)).into_future(),
        }
    }
}

/// Send http/1 error response
pub struct SendError<T, R, E, C>(PhantomData<(T, R, E, C)>);

impl<T, R, E, C> Default for SendError<T, R, E, C>
where
    T: AsyncRead + AsyncWrite,
    E: ResponseError,
{
    fn default() -> Self {
        SendError(PhantomData)
    }
}

impl<T, R, E, C> NewService for SendError<T, R, E, C>
where
    T: AsyncRead + AsyncWrite + 'static,
    R: 'static,
    E: ResponseError + 'static,
{
    type Config = C;
    type Request = Result<R, (E, Framed<T, Codec>)>;
    type Response = R;
    type Error = (E, Framed<T, Codec>);
    type InitError = ();
    type Service = SendError<T, R, E, C>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self, _: &C) -> Self::Future {
        ok(SendError(PhantomData))
    }
}

impl<T, R, E, C> Service for SendError<T, R, E, C>
where
    T: AsyncRead + AsyncWrite + 'static,
    R: 'static,
    E: ResponseError + 'static,
{
    type Request = Result<R, (E, Framed<T, Codec>)>;
    type Response = R;
    type Error = (E, Framed<T, Codec>);
    type Future = Either<FutureResult<R, (E, Framed<T, Codec>)>, SendErrorFut<T, R, E>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Result<R, (E, Framed<T, Codec>)>) -> Self::Future {
        match req {
            Ok(r) => Either::A(ok(r)),
            Err((e, framed)) => {
                let res = e.error_response().drop_body();
                Either::B(SendErrorFut {
                    framed: Some(framed),
                    res: Some((res, BodySize::Empty).into()),
                    err: Some(e),
                    _t: PhantomData,
                })
            }
        }
    }
}

pub struct SendErrorFut<T, R, E> {
    res: Option<Message<(Response<()>, BodySize)>>,
    framed: Option<Framed<T, Codec>>,
    err: Option<E>,
    _t: PhantomData<R>,
}

impl<T, R, E> Future for SendErrorFut<T, R, E>
where
    E: ResponseError,
    T: AsyncRead + AsyncWrite,
{
    type Item = R;
    type Error = (E, Framed<T, Codec>);

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(res) = self.res.take() {
            if self.framed.as_mut().unwrap().force_send(res).is_err() {
                return Err((self.err.take().unwrap(), self.framed.take().unwrap()));
            }
        }
        match self.framed.as_mut().unwrap().poll_complete() {
            Ok(Async::Ready(_)) => {
                Err((self.err.take().unwrap(), self.framed.take().unwrap()))
            }
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(_) => Err((self.err.take().unwrap(), self.framed.take().unwrap())),
        }
    }
}
