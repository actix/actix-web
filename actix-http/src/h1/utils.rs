use actix_codec::{AsyncRead, AsyncWrite, Framed};
use futures::{Async, Future, Poll, Sink};

use crate::body::{BodySize, MessageBody, ResponseBody};
use crate::error::Error;
use crate::h1::{Codec, Message};
use crate::response::Response;

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
