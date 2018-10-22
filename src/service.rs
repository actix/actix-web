use std::io;
use std::marker::PhantomData;

use actix_net::codec::Framed;
use actix_net::service::{NewService, Service};
use futures::future::{ok, Either, FutureResult};
use futures::{Async, AsyncSink, Future, Poll, Sink};
use tokio_io::AsyncWrite;

use error::ResponseError;
use h1::{Codec, OutMessage};
use response::Response;

pub struct SendError<T, R, E>(PhantomData<(T, R, E)>);

impl<T, R, E> Default for SendError<T, R, E>
where
    T: AsyncWrite,
    E: ResponseError,
{
    fn default() -> Self {
        SendError(PhantomData)
    }
}

impl<T, R, E> NewService for SendError<T, R, E>
where
    T: AsyncWrite,
    E: ResponseError,
{
    type Request = Result<R, (E, Framed<T, Codec>)>;
    type Response = R;
    type Error = (E, Framed<T, Codec>);
    type InitError = ();
    type Service = SendError<T, R, E>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(SendError(PhantomData))
    }
}

impl<T, R, E> Service for SendError<T, R, E>
where
    T: AsyncWrite,
    E: ResponseError,
{
    type Request = Result<R, (E, Framed<T, Codec>)>;
    type Response = R;
    type Error = (E, Framed<T, Codec>);
    type Future = Either<FutureResult<R, (E, Framed<T, Codec>)>, SendErrorFut<T, R, E>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        match req {
            Ok(r) => Either::A(ok(r)),
            Err((e, framed)) => Either::B(SendErrorFut {
                framed: Some(framed),
                res: Some(OutMessage::Response(e.error_response())),
                err: Some(e),
                _t: PhantomData,
            }),
        }
    }
}

pub struct SendErrorFut<T, R, E> {
    res: Option<OutMessage>,
    framed: Option<Framed<T, Codec>>,
    err: Option<E>,
    _t: PhantomData<R>,
}

impl<T, R, E> Future for SendErrorFut<T, R, E>
where
    E: ResponseError,
    T: AsyncWrite,
{
    type Item = R;
    type Error = (E, Framed<T, Codec>);

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(res) = self.res.take() {
            match self.framed.as_mut().unwrap().start_send(res) {
                Ok(AsyncSink::Ready) => (),
                Ok(AsyncSink::NotReady(res)) => {
                    self.res = Some(res);
                    return Ok(Async::NotReady);
                }
                Err(_) => {
                    return Err((self.err.take().unwrap(), self.framed.take().unwrap()))
                }
            }
        }
        match self.framed.as_mut().unwrap().poll_complete() {
            Ok(Async::Ready(_)) => {
                return Err((self.err.take().unwrap(), self.framed.take().unwrap()))
            }
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(_) => {
                return Err((self.err.take().unwrap(), self.framed.take().unwrap()))
            }
        }
    }
}

pub struct SendResponse<T>(PhantomData<(T,)>);

impl<T> Default for SendResponse<T>
where
    T: AsyncWrite,
{
    fn default() -> Self {
        SendResponse(PhantomData)
    }
}

impl<T> NewService for SendResponse<T>
where
    T: AsyncWrite,
{
    type Request = (Response, Framed<T, Codec>);
    type Response = Framed<T, Codec>;
    type Error = io::Error;
    type InitError = ();
    type Service = SendResponse<T>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(SendResponse(PhantomData))
    }
}

impl<T> Service for SendResponse<T>
where
    T: AsyncWrite,
{
    type Request = (Response, Framed<T, Codec>);
    type Response = Framed<T, Codec>;
    type Error = io::Error;
    type Future = SendResponseFut<T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, (res, framed): Self::Request) -> Self::Future {
        SendResponseFut {
            res: Some(OutMessage::Response(res)),
            framed: Some(framed),
        }
    }
}

pub struct SendResponseFut<T> {
    res: Option<OutMessage>,
    framed: Option<Framed<T, Codec>>,
}

impl<T> Future for SendResponseFut<T>
where
    T: AsyncWrite,
{
    type Item = Framed<T, Codec>;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(res) = self.res.take() {
            match self.framed.as_mut().unwrap().start_send(res)? {
                AsyncSink::Ready => (),
                AsyncSink::NotReady(res) => {
                    self.res = Some(res);
                    return Ok(Async::NotReady);
                }
            }
        }
        match self.framed.as_mut().unwrap().poll_complete()? {
            Async::Ready(_) => Ok(Async::Ready(self.framed.take().unwrap())),
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}
