use std::future::Future;

use actix_http::{
    body::{BodySize, MessageBody},
    header::HeaderMap,
    Payload, RequestHeadType, ResponseHead,
};
use actix_utils::future::poll_fn;
use bytes::Bytes;
use h2::{
    client::{Builder, Connection, SendRequest},
    SendStream,
};
use http::{
    header::{HeaderValue, CONNECTION, CONTENT_LENGTH, HOST, TRANSFER_ENCODING},
    request::Request,
    Method, Version,
};
use log::trace;

use super::{
    config::ConnectorConfig,
    connection::{ConnectionIo, H2Connection},
    error::SendRequestError,
};
use crate::BoxError;

pub(crate) async fn send_request<Io, B>(
    mut io: H2Connection<Io>,
    head: RequestHeadType,
    body: B,
) -> Result<(ResponseHead, Payload), SendRequestError>
where
    Io: ConnectionIo,
    B: MessageBody,
    B::Error: Into<BoxError>,
{
    trace!("Sending client request: {:?} {:?}", head, body.size());

    let head_req = head.as_ref().method == Method::HEAD;
    let length = body.size();
    let eof = matches!(length, BodySize::None | BodySize::Sized(0));

    let mut req = Request::new(());
    *req.uri_mut() = head.as_ref().uri.clone();
    *req.method_mut() = head.as_ref().method.clone();
    *req.version_mut() = Version::HTTP_2;

    let mut skip_len = true;
    // let mut has_date = false;

    // Content length
    let _ = match length {
        BodySize::None => None,

        BodySize::Sized(0) => {
            #[allow(clippy::declare_interior_mutable_const)]
            const HV_ZERO: HeaderValue = HeaderValue::from_static("0");
            req.headers_mut().insert(CONTENT_LENGTH, HV_ZERO)
        }

        BodySize::Sized(len) => {
            let mut buf = itoa::Buffer::new();

            req.headers_mut().insert(
                CONTENT_LENGTH,
                HeaderValue::from_str(buf.format(len)).unwrap(),
            )
        }

        BodySize::Stream => {
            skip_len = false;
            None
        }
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
            // TODO: consider skipping other headers according to:
            //       https://datatracker.ietf.org/doc/html/rfc7540#section-8.1.2.2
            // omit HTTP/1.x only headers
            CONNECTION | TRANSFER_ENCODING | HOST => continue,
            CONTENT_LENGTH if skip_len => continue,
            // DATE => has_date = true,
            _ => {}
        }
        req.headers_mut().append(key, value.clone());
    }

    let res = poll_fn(|cx| io.poll_ready(cx)).await;
    if let Err(err) = res {
        io.on_release(err.is_io());
        return Err(SendRequestError::from(err));
    }

    let resp = match io.send_request(req, eof) {
        Ok((fut, send)) => {
            io.on_release(false);

            if !eof {
                send_body(body, send).await?;
            }
            fut.await.map_err(SendRequestError::from)?
        }
        Err(err) => {
            io.on_release(err.is_io());
            return Err(err.into());
        }
    };

    let (parts, body) = resp.into_parts();
    let payload = if head_req { Payload::None } else { body.into() };

    let mut head = ResponseHead::new(parts.status);
    head.version = parts.version;
    head.headers = parts.headers.into();
    Ok((head, payload))
}

async fn send_body<B>(body: B, mut send: SendStream<Bytes>) -> Result<(), SendRequestError>
where
    B: MessageBody,
    B::Error: Into<BoxError>,
{
    let mut buf = None;

    actix_rt::pin!(body);

    loop {
        if buf.is_none() {
            match poll_fn(|cx| body.as_mut().poll_next(cx)).await {
                Some(Ok(b)) => {
                    send.reserve_capacity(b.len());
                    buf = Some(b);
                }
                Some(Err(err)) => return Err(SendRequestError::Body(err.into())),
                None => {
                    if let Err(err) = send.send_data(Bytes::new(), true) {
                        return Err(err.into());
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

                if let Err(err) = send.send_data(bytes, false) {
                    return Err(err.into());
                }
                if !b.is_empty() {
                    send.reserve_capacity(b.len());
                } else {
                    buf = None;
                }
                continue;
            }
            Some(Err(err)) => return Err(err.into()),
        }
    }
}

pub(crate) fn handshake<Io: ConnectionIo>(
    io: Io,
    config: &ConnectorConfig,
) -> impl Future<Output = Result<(SendRequest<Bytes>, Connection<Io, Bytes>), h2::Error>> {
    let mut builder = Builder::new();
    builder
        .initial_window_size(config.stream_window_size)
        .initial_connection_window_size(config.conn_window_size)
        .enable_push(false);
    builder.handshake(io)
}
