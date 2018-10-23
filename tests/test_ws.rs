extern crate actix;
extern crate actix_http;
extern crate actix_net;
extern crate actix_web;
extern crate bytes;
extern crate futures;

use std::{io, thread};

use actix::System;
use actix_net::codec::Framed;
use actix_net::framed::IntoFramed;
use actix_net::server::Server;
use actix_net::service::{NewServiceExt, Service};
use actix_net::stream::TakeItem;
use actix_web::{test, ws as web_ws};
use bytes::{Bytes, BytesMut};
use futures::future::{lazy, ok, Either};
use futures::{Future, IntoFuture, Sink, Stream};

use actix_http::{h1, ws, ResponseError, ServiceConfig};

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
    let addr = test::TestServer::unused_addr();
    thread::spawn(move || {
        Server::new()
            .bind("test", addr, move || {
                IntoFramed::new(|| h1::Codec::new(ServiceConfig::default()))
                    .and_then(TakeItem::new().map_err(|_| ()))
                    .and_then(|(req, framed): (_, Framed<_, _>)| {
                        // validate request
                        if let Some(h1::Message::Item(req)) = req {
                            match ws::verify_handshake(&req) {
                                Err(e) => {
                                    // validation failed
                                    let resp = e.error_response();
                                    Either::A(
                                        framed
                                            .send(h1::Message::Item(resp))
                                            .map_err(|_| ())
                                            .map(|_| ()),
                                    )
                                }
                                Ok(_) => Either::B(
                                    // send response
                                    framed
                                        .send(h1::Message::Item(
                                            ws::handshake_response(&req).finish(),
                                        )).map_err(|_| ())
                                        .and_then(|framed| {
                                            // start websocket service
                                            let framed =
                                                framed.into_framed(ws::Codec::new());
                                            ws::Transport::with(framed, ws_service)
                                                .map_err(|_| ())
                                        }),
                                ),
                            }
                        } else {
                            panic!()
                        }
                    })
            }).unwrap()
            .run();
    });

    let mut sys = System::new("test");
    {
        let (reader, mut writer) = sys
            .block_on(web_ws::Client::new(format!("http://{}/", addr)).connect())
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

    // client service
    let mut client = sys
        .block_on(lazy(|| Ok::<_, ()>(ws::Client::default()).into_future()))
        .unwrap();
    let framed = sys
        .block_on(client.call(ws::Connect::new(format!("http://{}/", addr))))
        .unwrap();

    let framed = sys
        .block_on(framed.send(ws::Message::Text("text".to_string())))
        .unwrap();
    let (item, framed) = sys.block_on(framed.into_future()).map_err(|_| ()).unwrap();
    assert_eq!(item, Some(ws::Frame::Text(Some(BytesMut::from("text")))));

    let framed = sys
        .block_on(framed.send(ws::Message::Binary("text".into())))
        .unwrap();
    let (item, framed) = sys.block_on(framed.into_future()).map_err(|_| ()).unwrap();
    assert_eq!(
        item,
        Some(ws::Frame::Binary(Some(Bytes::from_static(b"text").into())))
    );

    let framed = sys
        .block_on(framed.send(ws::Message::Ping("text".into())))
        .unwrap();
    let (item, framed) = sys.block_on(framed.into_future()).map_err(|_| ()).unwrap();
    assert_eq!(item, Some(ws::Frame::Pong("text".to_string().into())));

    let framed = sys
        .block_on(framed.send(ws::Message::Close(Some(ws::CloseCode::Normal.into()))))
        .unwrap();

    let (item, _framed) = sys.block_on(framed.into_future()).map_err(|_| ()).unwrap();
    assert_eq!(
        item,
        Some(ws::Frame::Close(Some(ws::CloseCode::Normal.into())))
    )
}
