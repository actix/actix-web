use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use actix_codec::{AsyncRead, AsyncWrite, Framed};

use crate::body::{BodySize, MessageBody, ResponseBody};
use crate::error::Error;
use crate::h1::{Codec, Message};
use crate::response::Response;

/// Send http/1 response
#[pin_project::pin_project]
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
    type Output = Result<Framed<T, Codec>, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        loop {
            let mut body_ready = this.body.is_some();
            let framed = this.framed.as_mut().unwrap();

            // send body
            if this.res.is_none() && this.body.is_some() {
                while body_ready && this.body.is_some() && !framed.is_write_buf_full() {
                    match this.body.as_mut().unwrap().poll_next(cx)? {
                        Poll::Ready(item) => {
                            // body is done
                            if item.is_none() {
                                let _ = this.body.take();
                            }
                            framed.write(Message::Chunk(item))?;
                        }
                        Poll::Pending => body_ready = false,
                    }
                }
            }

            // flush write buffer
            if !framed.is_write_buf_empty() {
                match framed.flush(cx)? {
                    Poll::Ready(_) => {
                        if body_ready {
                            continue;
                        } else {
                            return Poll::Pending;
                        }
                    }
                    Poll::Pending => return Poll::Pending,
                }
            }

            // send response
            if let Some(res) = this.res.take() {
                framed.write(res)?;
                continue;
            }

            if this.body.is_some() {
                if body_ready {
                    continue;
                } else {
                    return Poll::Pending;
                }
            } else {
                break;
            }
        }
        Poll::Ready(Ok(this.framed.take().unwrap()))
    }
}
