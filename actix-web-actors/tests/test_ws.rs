use actix::prelude::*;
use actix_http::HttpService;
use actix_http_test::TestServer;
use actix_web::{web, App, HttpRequest};
use actix_web_actors::*;
use bytes::{Bytes, BytesMut};
use futures::{Sink, Stream};

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
            ws::Message::Close(reason) => ctx.close(reason),
            _ => (),
        }
    }
}

#[test]
fn test_simple() {
    let mut srv =
        TestServer::new(|| {
            HttpService::new(App::new().service(web::resource("/").to(
                |req: HttpRequest, stream: web::Payload| ws::start(Ws, &req, stream),
            )))
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
