extern crate actix;
extern crate actix_web;
extern crate futures;
extern crate http;
extern crate bytes;

use bytes::Bytes;
use futures::Stream;

use actix_web::*;
use actix::prelude::*;

struct Ws;

impl Actor for Ws {
    type Context = ws::WebsocketContext<Self>;
}

impl Handler<ws::Message> for Ws {
    type Result = ();

    fn handle(&mut self, msg: ws::Message, ctx: &mut Self::Context) {
        match msg {
            ws::Message::Ping(msg) => ctx.pong(&msg),
            ws::Message::Text(text) => ctx.text(&text),
            ws::Message::Binary(bin) => ctx.binary(bin),
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
    let (item, _) = srv.execute(reader.into_future()).unwrap();
    assert_eq!(item, Some(ws::Message::Pong("ping".to_owned())));
}
