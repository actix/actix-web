use std::{
    cell::Cell,
    convert::Infallible,
    task::{Context, Poll},
};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_http::{
    body::{BodySize, BoxBody},
    h1,
    ws::{self, CloseCode, Frame, Item, Message},
    Error, HttpService, Request, Response,
};
use actix_http_test::test_server;
use actix_service::{fn_factory, Service};
use bytes::{Bytes, BytesMut};
use derive_more::{Display, Error, From};
use futures_core::future::LocalBoxFuture;
use futures_util::{SinkExt as _, StreamExt as _};
use tokio_util::codec::Decoder as _;

#[derive(Clone)]
struct WsService(Cell<bool>);

impl WsService {
    fn new() -> Self {
        WsService(Cell::new(false))
    }

    fn set_polled(&self) {
        self.0.set(true);
    }

    fn was_polled(&self) -> bool {
        self.0.get()
    }
}

#[derive(Debug, Display, Error, From)]
enum WsServiceError {
    #[display("HTTP error")]
    Http(actix_http::Error),

    #[display("WS handshake error")]
    Ws(actix_http::ws::HandshakeError),

    #[display("I/O error")]
    Io(std::io::Error),

    #[display("dispatcher error")]
    Dispatcher,
}

impl From<WsServiceError> for Response<BoxBody> {
    fn from(err: WsServiceError) -> Self {
        match err {
            WsServiceError::Http(err) => err.into(),
            WsServiceError::Ws(err) => err.into(),
            WsServiceError::Io(_err) => unreachable!(),
            WsServiceError::Dispatcher => {
                Response::internal_server_error().set_body(BoxBody::new(format!("{}", err)))
            }
        }
    }
}

impl<T> Service<(Request, Framed<T, h1::Codec>)> for WsService
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Response = ();
    type Error = WsServiceError;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.set_polled();
        Poll::Ready(Ok(()))
    }

    fn call(&self, (req, mut framed): (Request, Framed<T, h1::Codec>)) -> Self::Future {
        assert!(self.was_polled());

        Box::pin(async move {
            let res = ws::handshake(req.head())?.message_body(())?;

            framed.send((res, BodySize::None).into()).await?;

            let framed = framed.replace_codec(ws::Codec::new());

            ws::Dispatcher::with(framed, service)
                .await
                .map_err(|_| WsServiceError::Dispatcher)?;

            Ok(())
        })
    }
}

async fn service(msg: Frame) -> Result<Message, Error> {
    let msg = match msg {
        Frame::Ping(msg) => Message::Pong(msg),
        Frame::Text(text) => Message::Text(String::from_utf8_lossy(&text).into_owned().into()),
        Frame::Binary(bin) => Message::Binary(bin),
        Frame::Continuation(item) => Message::Continuation(item),
        Frame::Close(reason) => Message::Close(reason),
        _ => return Err(ws::ProtocolError::BadOpCode.into()),
    };

    Ok(msg)
}

fn masked_close(payload: &[u8]) -> BytesMut {
    let mask = [1, 2, 3, 4];
    let mut frame = BytesMut::with_capacity(6 + payload.len());
    frame.extend_from_slice(&[0x88, 0x80 | payload.len() as u8]);
    frame.extend_from_slice(&mask);

    for (idx, byte) in payload.iter().enumerate() {
        frame.extend_from_slice(&[byte ^ mask[idx % mask.len()]]);
    }

    frame
}

#[test]
fn rejects_invalid_close_codes() {
    for code in [0_u16, 999, 1004, 1005, 1006, 1015, 1016, 1100, 2000, 2999] {
        let mut codec = ws::Codec::new();
        let mut frame = masked_close(&code.to_be_bytes());

        assert!(matches!(
            codec.decode(&mut frame).unwrap_err(),
            ws::ProtocolError::BadOpCode
        ));
    }
}

#[test]
fn rejects_invalid_close_reason() {
    let mut codec = ws::Codec::new();
    let mut frame = masked_close(&[0x03, 0xe8, 0xff]);

    assert!(matches!(
        codec.decode(&mut frame).unwrap_err(),
        ws::ProtocolError::BadOpCode
    ));
}

#[test]
fn accepts_valid_close_codes() {
    for code in [
        1000_u16, 1001, 1002, 1003, 1007, 1008, 1009, 1010, 1011, 1012, 1013, 1014, 3000, 3999,
        4999,
    ] {
        let mut codec = ws::Codec::new();
        let mut frame = masked_close(&code.to_be_bytes());

        let Some(Frame::Close(Some(reason))) = codec.decode(&mut frame).unwrap() else {
            panic!("expected close reason for status code {code}");
        };
        assert_eq!(u16::from(reason.code), code);
        assert_eq!(reason.description, None);
    }
}

#[test]
fn rejects_close_payload_without_status_code() {
    let mut codec = ws::Codec::new();
    let mut frame = masked_close(&[0]);

    assert!(matches!(
        codec.decode(&mut frame).unwrap_err(),
        ws::ProtocolError::InvalidLength(1)
    ));
}

#[test]
fn accepts_close_payload_without_status_code() {
    let mut codec = ws::Codec::new();
    let mut frame = masked_close(&[]);

    assert_eq!(codec.decode(&mut frame).unwrap(), Some(Frame::Close(None)));
}

#[test]
fn decodes_valid_close_reason() {
    let mut codec = ws::Codec::new();
    let mut frame = masked_close(&[0x03, 0xe8, b'f', b'o', b'o']);

    let Some(Frame::Close(Some(reason))) = codec.decode(&mut frame).unwrap() else {
        panic!("expected close reason");
    };
    assert_eq!(reason.code, CloseCode::Normal);
    assert_eq!(reason.description.as_deref(), Some("foo"));
}

#[expect(deprecated)]
#[test]
fn legacy_close_payload_parser_wrongly_allows_non_utf8() {
    assert!(ws::Parser::parse_close_payload(&[0]).is_none());

    let reason = ws::Parser::parse_close_payload(&[0, 0, 0xff]).expect("expected close reason");
    assert_eq!(u16::from(reason.code), 0);
    assert_eq!(reason.description.as_deref(), Some("\u{fffd}"));
}

#[actix_rt::test]
async fn simple() {
    let mut srv = test_server(|| {
        HttpService::build()
            .upgrade(fn_factory(|| async {
                Ok::<_, Infallible>(WsService::new())
            }))
            .finish(|_| async { Ok::<_, Infallible>(Response::not_found()) })
            .tcp()
    })
    .await;

    // client service
    let mut framed = srv.ws().await.unwrap();
    framed.send(Message::Text("text".into())).await.unwrap();

    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, Frame::Text(Bytes::from_static(b"text")));

    framed.send(Message::Binary("text".into())).await.unwrap();

    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, Frame::Binary(Bytes::from_static(&b"text"[..])));

    framed.send(Message::Ping("text".into())).await.unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, Frame::Pong("text".to_string().into()));

    framed
        .send(Message::Continuation(Item::FirstText("text".into())))
        .await
        .unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(
        item,
        Frame::Continuation(Item::FirstText(Bytes::from_static(b"text")))
    );

    assert!(framed
        .send(Message::Continuation(Item::FirstText("text".into())))
        .await
        .is_err());
    assert!(framed
        .send(Message::Continuation(Item::FirstBinary("text".into())))
        .await
        .is_err());

    framed
        .send(Message::Continuation(Item::Continue("text".into())))
        .await
        .unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(
        item,
        Frame::Continuation(Item::Continue(Bytes::from_static(b"text")))
    );

    framed
        .send(Message::Continuation(Item::Last("text".into())))
        .await
        .unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(
        item,
        Frame::Continuation(Item::Last(Bytes::from_static(b"text")))
    );

    assert!(framed
        .send(Message::Continuation(Item::Continue("text".into())))
        .await
        .is_err());

    assert!(framed
        .send(Message::Continuation(Item::Last("text".into())))
        .await
        .is_err());

    framed
        .send(Message::Close(Some(CloseCode::Normal.into())))
        .await
        .unwrap();

    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, Frame::Close(Some(CloseCode::Normal.into())));
}

#[test]
fn rejects_new_data_frame_during_continuation() {
    let mut codec = ws::Codec::new();
    let mut buf = BytesMut::new();

    ws::Parser::write_message(&mut buf, b"fragment1", ws::OpCode::Text, false, true);
    ws::Parser::write_message(&mut buf, b"fragment2", ws::OpCode::Text, true, true);

    assert_eq!(
        codec.decode(&mut buf).unwrap(),
        Some(Frame::Continuation(Item::FirstText(Bytes::from_static(
            b"fragment1",
        )))),
        "the first text fragment should be returned as a continuation"
    );
    assert!(
        matches!(
            codec.decode(&mut buf),
            Err(ws::ProtocolError::ContinuationStarted)
        ),
        "a final text frame should be rejected during a continuation"
    );
}

#[test]
fn rejects_new_binary_frame_during_continuation() {
    let mut codec = ws::Codec::new();
    let mut buf = BytesMut::new();

    ws::Parser::write_message(&mut buf, b"fragment1", ws::OpCode::Binary, false, true);
    ws::Parser::write_message(&mut buf, b"fragment2", ws::OpCode::Binary, true, true);

    assert_eq!(
        codec.decode(&mut buf).unwrap(),
        Some(Frame::Continuation(Item::FirstBinary(Bytes::from_static(
            b"fragment1",
        )))),
        "the first binary fragment should be returned as a continuation"
    );
    assert!(
        matches!(
            codec.decode(&mut buf),
            Err(ws::ProtocolError::ContinuationStarted)
        ),
        "a final binary frame should be rejected during a continuation"
    );
}
