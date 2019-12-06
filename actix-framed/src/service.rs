use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_http::body::BodySize;
use actix_http::error::ResponseError;
use actix_http::h1::{Codec, Message};
use actix_http::ws::{verify_handshake, HandshakeError};
use actix_http::{Request, Response};
use actix_service::{Service, ServiceFactory};
use futures::future::{err, ok, Either, Ready};
use futures::Future;

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

impl<T, C> ServiceFactory for VerifyWebSockets<T, C> {
    type Config = C;
    type Request = (Request, Framed<T, Codec>);
    type Response = (Request, Framed<T, Codec>);
    type Error = (HandshakeError, Framed<T, Codec>);
    type InitError = ();
    type Service = VerifyWebSockets<T, C>;
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: C) -> Self::Future {
        ok(VerifyWebSockets { _t: PhantomData })
    }
}

impl<T, C> Service for VerifyWebSockets<T, C> {
    type Request = (Request, Framed<T, Codec>);
    type Response = (Request, Framed<T, Codec>);
    type Error = (HandshakeError, Framed<T, Codec>);
    type Future = Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, (req, framed): (Request, Framed<T, Codec>)) -> Self::Future {
        match verify_handshake(req.head()) {
            Err(e) => err((e, framed)),
            Ok(_) => ok((req, framed)),
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

impl<T, R, E, C> ServiceFactory for SendError<T, R, E, C>
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
    R: 'static,
    E: ResponseError + 'static,
{
    type Config = C;
    type Request = Result<R, (E, Framed<T, Codec>)>;
    type Response = R;
    type Error = (E, Framed<T, Codec>);
    type InitError = ();
    type Service = SendError<T, R, E, C>;
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: C) -> Self::Future {
        ok(SendError(PhantomData))
    }
}

impl<T, R, E, C> Service for SendError<T, R, E, C>
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
    R: 'static,
    E: ResponseError + 'static,
{
    type Request = Result<R, (E, Framed<T, Codec>)>;
    type Response = R;
    type Error = (E, Framed<T, Codec>);
    type Future = Either<Ready<Result<R, (E, Framed<T, Codec>)>>, SendErrorFut<T, R, E>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Result<R, (E, Framed<T, Codec>)>) -> Self::Future {
        match req {
            Ok(r) => Either::Left(ok(r)),
            Err((e, framed)) => {
                let res = e.error_response().drop_body();
                Either::Right(SendErrorFut {
                    framed: Some(framed),
                    res: Some((res, BodySize::Empty).into()),
                    err: Some(e),
                    _t: PhantomData,
                })
            }
        }
    }
}

#[pin_project::pin_project]
pub struct SendErrorFut<T, R, E> {
    res: Option<Message<(Response<()>, BodySize)>>,
    framed: Option<Framed<T, Codec>>,
    err: Option<E>,
    _t: PhantomData<R>,
}

impl<T, R, E> Future for SendErrorFut<T, R, E>
where
    E: ResponseError,
    T: AsyncRead + AsyncWrite + Unpin,
{
    type Output = Result<R, (E, Framed<T, Codec>)>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        if let Some(res) = self.res.take() {
            if self.framed.as_mut().unwrap().write(res).is_err() {
                return Poll::Ready(Err((
                    self.err.take().unwrap(),
                    self.framed.take().unwrap(),
                )));
            }
        }
        match self.framed.as_mut().unwrap().flush(cx) {
            Poll::Ready(Ok(_)) => {
                Poll::Ready(Err((self.err.take().unwrap(), self.framed.take().unwrap())))
            }
            Poll::Ready(Err(_)) => {
                Poll::Ready(Err((self.err.take().unwrap(), self.framed.take().unwrap())))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
