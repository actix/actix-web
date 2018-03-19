extern crate actix;
extern crate actix_web;
extern crate futures;
extern crate http;
extern crate bytes;
extern crate rand;

use bytes::Bytes;
use futures::Stream;
use rand::Rng;

use actix_web::*;
use actix::prelude::*;

struct Ws;

impl Actor for Ws {
    type Context = ws::WebsocketContext<Self>;
}

impl StreamHandler<ws::Message, ws::ProtocolError> for Ws {

    fn handle(&mut self, msg: ws::Message, ctx: &mut Self::Context) {
        match msg {
            ws::Message::Ping(msg) => ctx.pong(&msg),
            ws::Message::Text(text) => ctx.text(text),
            ws::Message::Binary(bin) => ctx.binary(bin),
            ws::Message::Close(reason) => ctx.close(reason, ""),
            _ => (),
        }
    }
}

#[test]
fn test_simple() {
    let mut srv = test::TestServer::new(
        |app| app.handler(|req| ws::start(req, Ws)));
    let (reader, mut writer) = srv.ws().unwrap();

    writer.text("text");
    let (item, reader) = srv.execute(reader.into_future()).unwrap();
    assert_eq!(item, Some(ws::Message::Text("text".to_owned())));

    writer.binary(b"text".as_ref());
    let (item, reader) = srv.execute(reader.into_future()).unwrap();
    assert_eq!(item, Some(ws::Message::Binary(Bytes::from_static(b"text").into())));

    writer.ping("ping");
    let (item, reader) = srv.execute(reader.into_future()).unwrap();
    assert_eq!(item, Some(ws::Message::Pong("ping".to_owned())));

    writer.close(ws::CloseCode::Normal, "");
    let (item, _) = srv.execute(reader.into_future()).unwrap();
    assert_eq!(item, Some(ws::Message::Close(ws::CloseCode::Normal)));
}

#[test]
fn test_large_text() {
    let data = rand::thread_rng()
        .gen_ascii_chars()
        .take(65_536)
        .collect::<String>();

    let mut srv = test::TestServer::new(
        |app| app.handler(|req| ws::start(req, Ws)));
    let (mut reader, mut writer) = srv.ws().unwrap();

    for _ in 0..100 {
        writer.text(data.clone());
        let (item, r) = srv.execute(reader.into_future()).unwrap();
        reader = r;
        assert_eq!(item, Some(ws::Message::Text(data.clone())));
    }
}

#[test]
fn test_large_bin() {
    let data = rand::thread_rng()
        .gen_ascii_chars()
        .take(65_536)
        .collect::<String>();

    let mut srv = test::TestServer::new(
        |app| app.handler(|req| ws::start(req, Ws)));
    let (mut reader, mut writer) = srv.ws().unwrap();

    for _ in 0..100 {
        writer.binary(data.clone());
        let (item, r) = srv.execute(reader.into_future()).unwrap();
        reader = r;
        assert_eq!(item, Some(ws::Message::Binary(Binary::from(data.clone()))));
    }
}
