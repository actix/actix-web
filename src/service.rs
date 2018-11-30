use std::marker::PhantomData;

use actix_net::codec::Framed;
use actix_net::service::{NewService, Service};
use futures::future::{ok, Either, FutureResult};
use futures::{Async, Future, Poll, Sink};
use tokio_io::{AsyncRead, AsyncWrite};

use body::{BodyLength, MessageBody, ResponseBody};
use error::{Error, ResponseError};
use h1::{Codec, Message};
use response::Response;

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

impl<T, R, E> NewService<Result<R, (E, Framed<T, Codec>)>> for SendError<T, R, E>
where
    T: AsyncRead + AsyncWrite,
    E: ResponseError,
{
    type Response = R;
    type Error = (E, Framed<T, Codec>);
    type InitError = ();
    type Service = SendError<T, R, E>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(SendError(PhantomData))
    }
}

impl<T, R, E> Service<Result<R, (E, Framed<T, Codec>)>> for SendError<T, R, E>
where
    T: AsyncRead + AsyncWrite,
    E: ResponseError,
{
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
                let mut res = e.error_response().set_body(format!("{}", e));
                let (res, _body) = res.replace_body(());
                Either::B(SendErrorFut {
                    framed: Some(framed),
                    res: Some((res, BodyLength::Empty).into()),
                    err: Some(e),
                    _t: PhantomData,
                })
            }
        }
    }
}

pub struct SendErrorFut<T, R, E> {
    res: Option<Message<(Response<()>, BodyLength)>>,
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
            if let Err(_) = self.framed.as_mut().unwrap().force_send(res) {
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

pub struct SendResponse<T, B>(PhantomData<(T, B)>);

impl<T, B> Default for SendResponse<T, B> {
    fn default() -> Self {
        SendResponse(PhantomData)
    }
}

impl<T, B> SendResponse<T, B>
where
    T: AsyncRead + AsyncWrite,
    B: MessageBody,
{
    pub fn send(
        framed: Framed<T, Codec>,
        res: Response<B>,
    ) -> impl Future<Item = Framed<T, Codec>, Error = Error> {
        // extract body from response
        let (res, body) = res.replace_body(());

        // write response
        SendResponseFut {
            res: Some(Message::Item((res, body.length()))),
            body: Some(body),
            framed: Some(framed),
        }
    }
}

impl<T, B> NewService<(Response<B>, Framed<T, Codec>)> for SendResponse<T, B>
where
    T: AsyncRead + AsyncWrite,
    B: MessageBody,
{
    type Response = Framed<T, Codec>;
    type Error = Error;
    type InitError = ();
    type Service = SendResponse<T, B>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(SendResponse(PhantomData))
    }
}

impl<T, B> Service<(Response<B>, Framed<T, Codec>)> for SendResponse<T, B>
where
    T: AsyncRead + AsyncWrite,
    B: MessageBody,
{
    type Response = Framed<T, Codec>;
    type Error = Error;
    type Future = SendResponseFut<T, B>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, (res, framed): (Response<B>, Framed<T, Codec>)) -> Self::Future {
        let (res, body) = res.replace_body(());
        SendResponseFut {
            res: Some(Message::Item((res, body.length()))),
            body: Some(body),
            framed: Some(framed),
        }
    }
}

pub struct SendResponseFut<T, B> {
    res: Option<Message<(Response<()>, BodyLength)>>,
    body: Option<ResponseBody<B>>,
    framed: Option<Framed<T, Codec>>,
}

impl<T, B> Future for SendResponseFut<T, B>
where
    T: AsyncRead + AsyncWrite,
    B: MessageBody,
{
    type Item = Framed<T, Codec>;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            let mut body_ready = self.body.is_some();
            let framed = self.framed.as_mut().unwrap();

            // send body
            if self.res.is_none() && self.body.is_some() {
                while body_ready && self.body.is_some() && !framed.is_write_buf_full() {
                    match self.body.as_mut().unwrap().poll_next()? {
                        Async::Ready(item) => {
                            // body is done
                            if item.is_none() {
                                let _ = self.body.take();
                            }
                            framed.force_send(Message::Chunk(item))?;
                        }
                        Async::NotReady => body_ready = false,
                    }
                }
            }

            // flush write buffer
            if !framed.is_write_buf_empty() {
                match framed.poll_complete()? {
                    Async::Ready(_) => if body_ready {
                        continue;
                    } else {
                        return Ok(Async::NotReady);
                    },
                    Async::NotReady => return Ok(Async::NotReady),
                }
            }

            // send response
            if let Some(res) = self.res.take() {
                framed.force_send(res)?;
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
        return Ok(Async::Ready(self.framed.take().unwrap()));
    }
}
