use std::io;

use actix_codec::Framed;
use actix_http::{body::BodySize, h1, ws, Error, HttpService, Request, Response};
use actix_http_test::TestServer;
use bytes::{Bytes, BytesMut};
use futures::future::ok;
use futures::{Future, Sink, Stream};

fn ws_service(req: ws::Frame) -> impl Future<Item = ws::Message, Error = io::Error> {
    match req {
        ws::Frame::Ping(msg) => ok(ws::Message::Pong(msg)),
        ws::Frame::Text(text) => {
            let text = if let Some(pl) = text {
                String::from_utf8(Vec::from(pl.as_ref())).unwrap()
            } else {
                String::new()
            };
            ok(ws::Message::Text(text))
        }
        ws::Frame::Binary(bin) => ok(ws::Message::Binary(
            bin.map(|e| e.freeze())
                .unwrap_or_else(|| Bytes::from(""))
                .into(),
        )),
        ws::Frame::Close(reason) => ok(ws::Message::Close(reason)),
        _ => ok(ws::Message::Close(None)),
    }
}

#[test]
fn test_simple() {
    let mut srv = TestServer::new(|| {
        HttpService::build()
            .upgrade(|(req, framed): (Request, Framed<_, _>)| {
                let res = ws::handshake_response(req.head()).finish();
                // send handshake response
                framed
                    .send(h1::Message::Item((res.drop_body(), BodySize::None)))
                    .map_err(|e: io::Error| e.into())
                    .and_then(|framed| {
                        // start websocket service
                        let framed = framed.into_framed(ws::Codec::new());
                        ws::Transport::with(framed, ws_service)
                    })
            })
            .finish(|_| ok::<_, Error>(Response::NotFound()))
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
