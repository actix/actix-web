use std::io;

use actix_codec::Framed;
use actix_http_test::TestServer;
use actix_service::NewService;
use actix_utils::framed::IntoFramed;
use actix_utils::stream::TakeItem;
use actix_web::ws as web_ws;
use bytes::{Bytes, BytesMut};
use futures::future::{lazy, ok, Either};
use futures::{Future, Sink, Stream};

use actix_http::{h1, ws, ResponseError, SendResponse, ServiceConfig};

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
    let mut srv = TestServer::with_factory(|| {
        IntoFramed::new(|| h1::Codec::new(ServiceConfig::default()))
            .and_then(TakeItem::new().map_err(|_| ()))
            .and_then(|(req, framed): (_, Framed<_, _>)| {
                // validate request
                if let Some(h1::Message::Item(req)) = req {
                    match ws::verify_handshake(&req) {
                        Err(e) => {
                            // validation failed
                            Either::A(
                                SendResponse::send(framed, e.error_response())
                                    .map_err(|_| ())
                                    .map(|_| ()),
                            )
                        }
                        Ok(_) => {
                            Either::B(
                                // send handshake response
                                SendResponse::send(
                                    framed,
                                    ws::handshake_response(&req).finish(),
                                )
                                .map_err(|_| ())
                                .and_then(|framed| {
                                    // start websocket service
                                    let framed = framed.into_framed(ws::Codec::new());
                                    ws::Transport::with(framed, ws_service)
                                        .map_err(|_| ())
                                }),
                            )
                        }
                    }
                } else {
                    panic!()
                }
            })
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

    {
        let mut sys = actix_web::actix::System::new("test");
        let url = srv.url("/");

        let (reader, mut writer) = sys
            .block_on(lazy(|| web_ws::Client::new(url).connect()))
            .unwrap();

        writer.text("text");
        let (item, reader) = sys.block_on(reader.into_future()).unwrap();
        assert_eq!(item, Some(web_ws::Message::Text("text".to_owned())));

        writer.binary(b"text".as_ref());
        let (item, reader) = sys.block_on(reader.into_future()).unwrap();
        assert_eq!(
            item,
            Some(web_ws::Message::Binary(Bytes::from_static(b"text").into()))
        );

        writer.ping("ping");
        let (item, reader) = sys.block_on(reader.into_future()).unwrap();
        assert_eq!(item, Some(web_ws::Message::Pong("ping".to_owned())));

        writer.close(Some(web_ws::CloseCode::Normal.into()));
        let (item, _) = sys.block_on(reader.into_future()).unwrap();
        assert_eq!(
            item,
            Some(web_ws::Message::Close(Some(
                web_ws::CloseCode::Normal.into()
            )))
        );
    }
}
