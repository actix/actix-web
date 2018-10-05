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
use actix_net::service::NewServiceExt;
use actix_web::{test, ws as web_ws};
use bytes::Bytes;
use futures::future::{ok, Either};
use futures::{Future, Sink, Stream};

use actix_http::{h1, ws, ResponseError};

fn ws_service(req: ws::Message) -> impl Future<Item = ws::Message, Error = io::Error> {
    match req {
        ws::Message::Ping(msg) => ok(ws::Message::Pong(msg)),
        ws::Message::Text(text) => ok(ws::Message::Text(text)),
        ws::Message::Binary(bin) => ok(ws::Message::Binary(bin)),
        ws::Message::Close(reason) => ok(ws::Message::Close(reason)),
        _ => ok(ws::Message::Close(None)),
    }
}

#[test]
fn test_simple() {
    let addr = test::TestServer::unused_addr();
    thread::spawn(move || {
        Server::new()
            .bind("test", addr, move || {
                IntoFramed::new(|| h1::Codec::new(false))
                    .and_then(h1::TakeRequest::new().map_err(|_| ()))
                    .and_then(|(req, framed): (_, Framed<_, _>)| {
                        // validate request
                        if let Some(h1::InMessage::MessageWithPayload(req)) = req {
                            match ws::handshake(&req) {
                                Err(e) => {
                                    // validation failed
                                    let resp = e.error_response();
                                    Either::A(
                                        framed
                                            .send(h1::OutMessage::Response(resp))
                                            .map_err(|_| ())
                                            .map(|_| ()),
                                    )
                                }
                                Ok(mut resp) => Either::B(
                                    // send response
                                    framed
                                        .send(h1::OutMessage::Response(resp.finish()))
                                        .map_err(|_| ())
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
}
