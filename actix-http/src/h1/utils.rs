use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use actix_codec::{AsyncRead, AsyncWrite, Framed};

use crate::body::{BodySize, MessageBody, ResponseBody};
use crate::error::Error;
use crate::h1::{Codec, Message};
use crate::response::Response;

/// Send HTTP/1 response
#[pin_project::pin_project]
pub struct SendResponse<T, B> {
    res: Option<Message<(Response<()>, BodySize)>>,
    #[pin]
    body: Option<ResponseBody<B>>,
    #[pin]
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
    T: AsyncRead + AsyncWrite + Unpin,
    B: MessageBody + Unpin,
{
    type Output = Result<Framed<T, Codec>, Error>;

    // TODO: rethink if we need loops in polls
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.as_mut().project();

        let mut body_done = this.body.is_none();
        loop {
            let mut body_ready = !body_done;

            // send body
            if this.res.is_none() && body_ready {
                while body_ready
                    && !body_done
                    && !this
                        .framed
                        .as_ref()
                        .as_pin_ref()
                        .unwrap()
                        .is_write_buf_full()
                {
                    match this.body.as_mut().as_pin_mut().unwrap().poll_next(cx)? {
                        Poll::Ready(item) => {
                            // body is done when item is None
                            body_done = item.is_none();
                            if body_done {
                                let _ = this.body.take();
                            }
                            let framed = this.framed.as_mut().as_pin_mut().unwrap();
                            framed.write(Message::Chunk(item))?;
                        }
                        Poll::Pending => body_ready = false,
                    }
                }
            }

            let framed = this.framed.as_mut().as_pin_mut().unwrap();

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

            if !body_done {
                if body_ready {
                    continue;
                } else {
                    return Poll::Pending;
                }
            } else {
                break;
            }
        }

        let framed = this.framed.take().unwrap();

        Poll::Ready(Ok(framed))
    }
}
