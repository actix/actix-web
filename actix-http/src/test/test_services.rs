use std::{
    cell::Cell,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use actix_service::{fn_service, Service};
use actix_utils::future::ready;
use bytes::{Buf, Bytes, BytesMut};

use crate::{body::MessageBody, Error, Request, Response, StatusCode};

pub(crate) struct ReadyChunkBody {
    chunk_polls: Rc<Cell<usize>>,
    remaining: usize,
    chunk_len: usize,
}

impl ReadyChunkBody {
    fn new(chunk_polls: Rc<Cell<usize>>, remaining: usize, chunk_len: usize) -> Self {
        Self {
            chunk_polls,
            remaining,
            chunk_len,
        }
    }
}

impl MessageBody for ReadyChunkBody {
    type Error = Error;

    fn size(&self) -> crate::body::BodySize {
        crate::body::BodySize::Stream
    }

    fn poll_next(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        if self.remaining == 0 {
            return Poll::Ready(None);
        }

        self.remaining -= 1;
        self.chunk_polls.set(self.chunk_polls.get() + 1);

        Poll::Ready(Some(Ok(Bytes::from(vec![b'x'; self.chunk_len]))))
    }
}

/// Creates a service that responds with an empty `200 OK` response.
pub(crate) fn ok_service(
) -> impl Service<Request, Response = Response<impl MessageBody>, Error = Error> {
    status_service(StatusCode::OK)
}

/// Creates a service that responds with the given status and an empty body.
fn status_service(
    status: StatusCode,
) -> impl Service<Request, Response = Response<impl MessageBody>, Error = Error> {
    fn_service(move |_req: Request| ready(Ok::<_, Error>(Response::new(status))))
}

/// Creates a service that echoes the request path in the response body.
pub(crate) fn echo_path_service(
) -> impl Service<Request, Response = Response<impl MessageBody>, Error = Error> {
    fn_service(|req: Request| {
        let path = req.path().as_bytes();
        ready(Ok::<_, Error>(
            Response::ok().set_body(Bytes::copy_from_slice(path)),
        ))
    })
}

/// Creates a service that drops the request payload and returns a `200 OK` response.
pub(crate) fn drop_payload_service(
) -> impl Service<Request, Response = Response<&'static str>, Error = Error> {
    fn_service(|mut req: Request| async move {
        let _ = req.take_payload();
        Ok::<_, Error>(Response::with_body(StatusCode::OK, "payload dropped"))
    })
}

/// Creates a service that ignores the request payload and returns a `200 OK` response.
pub(crate) fn ignore_payload_service(
) -> impl Service<Request, Response = Response<&'static str>, Error = Error> {
    fn_service(|_req: Request| ready(Ok::<_, Error>(Response::with_body(StatusCode::OK, "ok"))))
}

/// Creates a service that echoes the request payload in the response body.
pub(crate) fn echo_payload_service(
) -> impl Service<Request, Response = Response<Bytes>, Error = Error> {
    fn_service(|mut req: Request| {
        Box::pin(async move {
            use futures_util::StreamExt as _;

            let mut pl = req.take_payload();
            let mut body = BytesMut::new();
            while let Some(chunk) = pl.next().await {
                body.extend_from_slice(chunk.unwrap().chunk())
            }

            Ok::<_, Error>(Response::ok().set_body(body.freeze()))
        })
    })
}

/// Creates a service that returns a configurable ready chunk body.
pub(crate) fn ready_chunk_body_service(
    chunk_polls: Rc<Cell<usize>>,
    chunk_count: usize,
    chunk_len: usize,
) -> impl Service<Request, Response = Response<ReadyChunkBody>, Error = Error> {
    fn_service(move |_req: Request| {
        ready(Ok::<_, Error>(Response::ok().set_body(
            ReadyChunkBody::new(chunk_polls.clone(), chunk_count, chunk_len),
        )))
    })
}

/// Creates a service that returns a `101 Switching Protocols` response.
pub(crate) fn upgrade_response_service(
) -> impl Service<Request, Response = Response<impl MessageBody>, Error = Error> {
    fn_service(|_req: Request| {
        ready(Ok::<_, Error>(
            Response::build(StatusCode::SWITCHING_PROTOCOLS)
                .upgrade("websocket")
                .finish(),
        ))
    })
}
