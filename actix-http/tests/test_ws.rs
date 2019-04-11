use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_http::{body, h1, ws, Error, HttpService, Request, Response};
use actix_http_test::TestServer;
use actix_utils::framed::FramedTransport;
use bytes::{Bytes, BytesMut};
use futures::future::{self, ok};
use futures::{Future, Sink, Stream};

fn ws_service<T: AsyncRead + AsyncWrite>(
    (req, framed): (Request, Framed<T, h1::Codec>),
) -> impl Future<Item = (), Error = Error> {
    let res = ws::handshake(req.head()).unwrap().message_body(());

    framed
        .send((res, body::BodySize::None).into())
        .map_err(|_| panic!())
        .and_then(|framed| {
            FramedTransport::new(framed.into_framed(ws::Codec::new()), service)
                .map_err(|_| panic!())
        })
}

fn service(msg: ws::Frame) -> impl Future<Item = ws::Message, Error = Error> {
    let msg = match msg {
        ws::Frame::Ping(msg) => ws::Message::Pong(msg),
        ws::Frame::Text(text) => {
            ws::Message::Text(String::from_utf8_lossy(&text.unwrap()).to_string())
        }
        ws::Frame::Binary(bin) => ws::Message::Binary(bin.unwrap().freeze()),
        ws::Frame::Close(reason) => ws::Message::Close(reason),
        _ => panic!(),
    };
    ok(msg)
}

#[test]
fn test_simple() {
    let mut srv = TestServer::new(|| {
        HttpService::build()
            .upgrade(ws_service)
            .finish(|_| future::ok::<_, ()>(Response::NotFound()))
    });

    // client service
    let framed = srv.ws().unwrap();
    let framed = srv
        .block_on(framed.send(ws::Message::Text("text".to_string())))
        .unwrap();
    let (item, framed) = srv.block_on(framed.into_future()).map_err(|_| ()).unwrap();
    assert_eq!(item, Some(ws::Frame::Text(Some(BytesMut::from("text")))));

    let framed = srv
        .block_on(framed.send(ws::Message::Binary("text".into())))
        .unwrap();
    let (item, framed) = srv.block_on(framed.into_future()).map_err(|_| ()).unwrap();
    assert_eq!(
        item,
        Some(ws::Frame::Binary(Some(Bytes::from_static(b"text").into())))
    );

    let framed = srv
        .block_on(framed.send(ws::Message::Ping("text".into())))
        .unwrap();
    let (item, framed) = srv.block_on(framed.into_future()).map_err(|_| ()).unwrap();
    assert_eq!(item, Some(ws::Frame::Pong("text".to_string().into())));

    let framed = srv
        .block_on(framed.send(ws::Message::Close(Some(ws::CloseCode::Normal.into()))))
        .unwrap();

    let (item, _framed) = srv.block_on(framed.into_future()).map_err(|_| ()).unwrap();
    assert_eq!(
        item,
        Some(ws::Frame::Close(Some(ws::CloseCode::Normal.into())))
    );
}
