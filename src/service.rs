use std::marker::PhantomData;

use actix_net::codec::Framed;
use actix_net::service::{NewService, Service};
use futures::future::{ok, Either, FutureResult};
use futures::{Async, AsyncSink, Future, Poll, Sink};
use tokio_io::{AsyncRead, AsyncWrite};

use body::Body;
use error::{Error, ResponseError};
use h1::{Codec, Message};
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
            Err((e, framed)) => {
                let mut resp = e.error_response();
                resp.set_body(format!("{}", e));
                Either::B(SendErrorFut {
                    framed: Some(framed),
                    res: Some(resp.into()),
                    err: Some(e),
                    _t: PhantomData,
                })
            }
        }
    }
}

pub struct SendErrorFut<T, R, E> {
    res: Option<Message<Response>>,
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
                Err((self.err.take().unwrap(), self.framed.take().unwrap()))
            }
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(_) => Err((self.err.take().unwrap(), self.framed.take().unwrap())),
        }
    }
}

pub struct SendResponse<T>(PhantomData<(T,)>);

impl<T> Default for SendResponse<T>
where
    T: AsyncRead + AsyncWrite,
{
    fn default() -> Self {
        SendResponse(PhantomData)
    }
}

impl<T> SendResponse<T>
where
    T: AsyncRead + AsyncWrite,
{
    pub fn send(
        mut framed: Framed<T, Codec>,
        mut res: Response,
    ) -> impl Future<Item = Framed<T, Codec>, Error = Error> {
        // init codec
        framed.get_codec_mut().prepare_te(&mut res);

        // extract body from response
        let body = res.replace_body(Body::Empty);

        // write response
        SendResponseFut {
            res: Some(Message::Item(res)),
            body: Some(body),
            framed: Some(framed),
        }
    }
}

impl<T> NewService for SendResponse<T>
where
    T: AsyncRead + AsyncWrite,
{
    type Request = (Response, Framed<T, Codec>);
    type Response = Framed<T, Codec>;
    type Error = Error;
    type InitError = ();
    type Service = SendResponse<T>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(SendResponse(PhantomData))
    }
}

impl<T> Service for SendResponse<T>
where
    T: AsyncRead + AsyncWrite,
{
    type Request = (Response, Framed<T, Codec>);
    type Response = Framed<T, Codec>;
    type Error = Error;
    type Future = SendResponseFut<T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, (mut res, mut framed): Self::Request) -> Self::Future {
        framed.get_codec_mut().prepare_te(&mut res);
        let body = res.replace_body(Body::Empty);
        SendResponseFut {
            res: Some(Message::Item(res)),
            body: Some(body),
            framed: Some(framed),
        }
    }
}

pub struct SendResponseFut<T> {
    res: Option<Message<Response>>,
    body: Option<Body>,
    framed: Option<Framed<T, Codec>>,
}

impl<T> Future for SendResponseFut<T>
where
    T: AsyncRead + AsyncWrite,
{
    type Item = Framed<T, Codec>;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        // send response
        if self.res.is_some() {
            let framed = self.framed.as_mut().unwrap();
            if !framed.is_write_buf_full() {
                if let Some(res) = self.res.take() {
                    framed.force_send(res)?;
                }
            }
        }

        // send body
        if self.res.is_none() && self.body.is_some() {
            let framed = self.framed.as_mut().unwrap();
            if !framed.is_write_buf_full() {
                let body = self.body.take().unwrap();
                match body {
                    Body::Empty => (),
                    Body::Streaming(mut stream) => loop {
                        match stream.poll()? {
                            Async::Ready(item) => {
                                let done = item.is_none();
                                framed.force_send(Message::Chunk(item.into()))?;
                                if !done {
                                    if !framed.is_write_buf_full() {
                                        continue;
                                    } else {
                                        self.body = Some(Body::Streaming(stream));
                                        break;
                                    }
                                }
                            }
                            Async::NotReady => {
                                self.body = Some(Body::Streaming(stream));
                                break;
                            }
                        }
                    },
                    Body::Binary(mut bin) => {
                        framed.force_send(Message::Chunk(Some(bin.take())))?;
                        framed.force_send(Message::Chunk(None))?;
                    }
                }
            }
        }

        // flush
        match self.framed.as_mut().unwrap().poll_complete()? {
            Async::Ready(_) => if self.res.is_some() || self.body.is_some() {
                return self.poll();
            },
            Async::NotReady => return Ok(Async::NotReady),
        }

        Ok(Async::Ready(self.framed.take().unwrap()))
    }
}
