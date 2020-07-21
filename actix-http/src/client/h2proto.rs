use std::convert::TryFrom;
use std::future::Future;
use std::time;

use actix_codec::{AsyncRead, AsyncWrite};
use bytes::Bytes;
use futures_util::future::poll_fn;
use futures_util::pin_mut;
use h2::{
    client::{Builder, Connection, SendRequest},
    SendStream,
};
use http::header::{HeaderValue, CONNECTION, CONTENT_LENGTH, TRANSFER_ENCODING};
use http::{request::Request, Method, Version};

use crate::body::{BodySize, MessageBody};
use crate::header::HeaderMap;
use crate::message::{RequestHeadType, ResponseHead};
use crate::payload::Payload;

use super::config::ConnectorConfig;
use super::connection::{ConnectionType, IoConnection};
use super::error::SendRequestError;
use super::pool::Acquired;

pub(crate) async fn send_request<T, B>(
    mut io: SendRequest<Bytes>,
    head: RequestHeadType,
    body: B,
    created: time::Instant,
    pool: Option<Acquired<T>>,
) -> Result<(ResponseHead, Payload), SendRequestError>
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
    B: MessageBody,
{
    trace!("Sending client request: {:?} {:?}", head, body.size());
    let head_req = head.as_ref().method == Method::HEAD;
    let length = body.size();
    let eof = matches!(
        length,
        BodySize::None | BodySize::Empty | BodySize::Sized(0)
    );

    let mut req = Request::new(());
    *req.uri_mut() = head.as_ref().uri.clone();
    *req.method_mut() = head.as_ref().method.clone();
    *req.version_mut() = Version::HTTP_2;

    let mut skip_len = true;
    // let mut has_date = false;

    // Content length
    let _ = match length {
        BodySize::None => None,
        BodySize::Stream => {
            skip_len = false;
            None
        }
        BodySize::Empty => req
            .headers_mut()
            .insert(CONTENT_LENGTH, HeaderValue::from_static("0")),
        BodySize::Sized(len) => req.headers_mut().insert(
            CONTENT_LENGTH,
            HeaderValue::try_from(format!("{}", len)).unwrap(),
        ),
    };

    // Extracting extra headers from RequestHeadType. HeaderMap::new() does not allocate.
    let (head, extra_headers) = match head {
        RequestHeadType::Owned(head) => (RequestHeadType::Owned(head), HeaderMap::new()),
        RequestHeadType::Rc(head, extra_headers) => (
            RequestHeadType::Rc(head, None),
            extra_headers.unwrap_or_else(HeaderMap::new),
        ),
    };

    // merging headers from head and extra headers.
    let headers = head
        .as_ref()
        .headers
        .iter()
        .filter(|(name, _)| !extra_headers.contains_key(*name))
        .chain(extra_headers.iter());

    // copy headers
    for (key, value) in headers {
        match *key {
            CONNECTION | TRANSFER_ENCODING => continue, // http2 specific
            CONTENT_LENGTH if skip_len => continue,
            // DATE => has_date = true,
            _ => (),
        }
        req.headers_mut().append(key, value.clone());
    }

    let res = poll_fn(|cx| io.poll_ready(cx)).await;
    if let Err(e) = res {
        release(io, pool, created, e.is_io());
        return Err(SendRequestError::from(e));
    }

    let resp = match io.send_request(req, eof) {
        Ok((fut, send)) => {
            release(io, pool, created, false);

            if !eof {
                send_body(body, send).await?;
            }
            fut.await.map_err(SendRequestError::from)?
        }
        Err(e) => {
            release(io, pool, created, e.is_io());
            return Err(e.into());
        }
    };

    let (parts, body) = resp.into_parts();
    let payload = if head_req { Payload::None } else { body.into() };

    let mut head = ResponseHead::new(parts.status);
    head.version = parts.version;
    head.headers = parts.headers.into();
    Ok((head, payload))
}

async fn send_body<B: MessageBody>(
    body: B,
    mut send: SendStream<Bytes>,
) -> Result<(), SendRequestError> {
    let mut buf = None;
    pin_mut!(body);
    loop {
        if buf.is_none() {
            match poll_fn(|cx| body.as_mut().poll_next(cx)).await {
                Some(Ok(b)) => {
                    send.reserve_capacity(b.len());
                    buf = Some(b);
                }
                Some(Err(e)) => return Err(e.into()),
                None => {
                    if let Err(e) = send.send_data(Bytes::new(), true) {
                        return Err(e.into());
                    }
                    send.reserve_capacity(0);
                    return Ok(());
                }
            }
        }

        match poll_fn(|cx| send.poll_capacity(cx)).await {
            None => return Ok(()),
            Some(Ok(cap)) => {
                let b = buf.as_mut().unwrap();
                let len = b.len();
                let bytes = b.split_to(std::cmp::min(cap, len));

                if let Err(e) = send.send_data(bytes, false) {
                    return Err(e.into());
                } else {
                    if !b.is_empty() {
                        send.reserve_capacity(b.len());
                    } else {
                        buf = None;
                    }
                    continue;
                }
            }
            Some(Err(e)) => return Err(e.into()),
        }
    }
}

// release SendRequest object
fn release<T: AsyncRead + AsyncWrite + Unpin + 'static>(
    io: SendRequest<Bytes>,
    pool: Option<Acquired<T>>,
    created: time::Instant,
    close: bool,
) {
    if let Some(mut pool) = pool {
        if close {
            pool.close(IoConnection::new(ConnectionType::H2(io), created, None));
        } else {
            pool.release(IoConnection::new(ConnectionType::H2(io), created, None));
        }
    }
}

pub(crate) fn handshake<Io>(
    io: Io,
    config: &ConnectorConfig,
) -> impl Future<Output = Result<(SendRequest<Bytes>, Connection<Io, Bytes>), h2::Error>>
where
    Io: AsyncRead + AsyncWrite + Unpin + 'static,
{
    let mut builder = Builder::new();
    builder
        .initial_window_size(config.stream_window_size)
        .initial_connection_window_size(config.conn_window_size)
        .enable_push(false);
    builder.handshake(io)
}
