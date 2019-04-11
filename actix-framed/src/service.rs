use std::marker::PhantomData;

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_http::body::{BodySize, MessageBody, ResponseBody};
use actix_http::error::{Error, ResponseError};
use actix_http::h1::{Codec, Message};
use actix_http::ws::{verify_handshake, HandshakeError};
use actix_http::{Request, Response};
use actix_service::{NewService, Service};
use futures::future::{ok, Either, FutureResult};
use futures::{Async, Future, IntoFuture, Poll, Sink};

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
    type Request = (Request, Framed<T, Codec>);
    type Response = (Request, Framed<T, Codec>);
    type Error = (HandshakeError, Framed<T, Codec>);
    type InitError = ();
    type Service = VerifyWebSockets<T>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self, _: &()) -> Self::Future {
        ok(VerifyWebSockets { _t: PhantomData })
    }
}

impl<T> Service for VerifyWebSockets<T> {
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
    type Request = Result<R, (E, Framed<T, Codec>)>;
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
    type Request = Result<R, (E, Framed<T, Codec>)>;
    type Response = R;
    type Error = Error;
    type Future = Either<FutureResult<R, Error>, Box<Future<Item = R, Error = Error>>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Result<R, (E, Framed<T, Codec>)>) -> Self::Future {
        match req {
            Ok(r) => Either::A(ok(r)),
            Err((e, framed)) => {
                let res = e.render_response();
                let e = Error::from(e);
                Either::B(Box::new(
                    SendResponse::new(framed, res).then(move |_| Err(e)),
                ))
            }
        }
    }
}

/// Send http/1 response
pub struct SendResponse<T, B> {
    res: Option<Message<(Response<()>, BodySize)>>,
    body: Option<ResponseBody<B>>,
    framed: Option<Framed<T, Codec>>,
}

impl<T, B> SendResponse<T, B>
where
    B: MessageBody,
{
    pub fn new(framed: Framed<T, Codec>, response: Response<B>) -> Self {
        let (res, body) = response.into_parts();

        SendResponse {
            res: Some((res, body.size()).into()),
            body: Some(body),
            framed: Some(framed),
        }
    }
}

impl<T, B> Future for SendResponse<T, B>
where
    T: AsyncRead + AsyncWrite,
    B: MessageBody,
{
    type Item = Framed<T, Codec>;
    type Error = (Error, Framed<T, Codec>);

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            let mut body_ready = self.body.is_some();

            // send body
            if self.res.is_none() && self.body.is_some() {
                while body_ready
                    && self.body.is_some()
                    && !self.framed.as_ref().unwrap().is_write_buf_full()
                {
                    match self
                        .body
                        .as_mut()
                        .unwrap()
                        .poll_next()
                        .map_err(|e| (e, self.framed.take().unwrap()))?
                    {
                        Async::Ready(item) => {
                            // body is done
                            if item.is_none() {
                                let _ = self.body.take();
                            }
                            self.framed
                                .as_mut()
                                .unwrap()
                                .force_send(Message::Chunk(item))
                                .map_err(|e| (e.into(), self.framed.take().unwrap()))?;
                        }
                        Async::NotReady => body_ready = false,
                    }
                }
            }

            // flush write buffer
            if !self.framed.as_ref().unwrap().is_write_buf_empty() {
                match self
                    .framed
                    .as_mut()
                    .unwrap()
                    .poll_complete()
                    .map_err(|e| (e.into(), self.framed.take().unwrap()))?
                {
                    Async::Ready(_) => {
                        if body_ready {
                            continue;
                        } else {
                            return Ok(Async::NotReady);
                        }
                    }
                    Async::NotReady => return Ok(Async::NotReady),
                }
            }

            // send response
            if let Some(res) = self.res.take() {
                self.framed
                    .as_mut()
                    .unwrap()
                    .force_send(res)
                    .map_err(|e| (e.into(), self.framed.take().unwrap()))?;
                continue;
            }

            if self.body.is_some() {
                if body_ready {
                    continue;
                } else {
                    return Ok(Async::NotReady);
                }
            } else {
                break;
            }
        }
        Ok(Async::Ready(self.framed.take().unwrap()))
    }
}
