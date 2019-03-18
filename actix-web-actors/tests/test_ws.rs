use actix::prelude::*;
use actix_http::HttpService;
use actix_http_test::TestServer;
use actix_web::{web, App, HttpRequest};
use actix_web_actors::*;
use bytes::{Bytes, BytesMut};
use futures::{Sink, Stream};

struct Ws;

impl Actor for Ws {
    type Context = WebsocketContext<Self>;
}

impl StreamHandler<WsFrame, WsProtocolError> for Ws {
    fn handle(&mut self, msg: WsFrame, ctx: &mut Self::Context) {
        match msg {
            WsFrame::Ping(msg) => ctx.pong(&msg),
            WsFrame::Text(text) => {
                ctx.text(String::from_utf8_lossy(&text.unwrap())).to_owned()
            }
            WsFrame::Binary(bin) => ctx.binary(bin.unwrap()),
            WsFrame::Close(reason) => ctx.close(reason),
            _ => (),
        }
    }
}

#[test]
fn test_simple() {
    let mut srv =
        TestServer::new(|| {
            HttpService::new(App::new().service(web::resource("/").to(
                |req: HttpRequest, stream: web::Payload<_>| ws_start(Ws, &req, stream),
            )))
        });

    // client service
    let framed = srv.ws().unwrap();
    let framed = srv
        .block_on(framed.send(WsMessage::Text("text".to_string())))
        .unwrap();
    let (item, framed) = srv.block_on(framed.into_future()).map_err(|_| ()).unwrap();
    assert_eq!(item, Some(WsFrame::Text(Some(BytesMut::from("text")))));

    let framed = srv
        .block_on(framed.send(WsMessage::Binary("text".into())))
        .unwrap();
    let (item, framed) = srv.block_on(framed.into_future()).map_err(|_| ()).unwrap();
    assert_eq!(
        item,
        Some(WsFrame::Binary(Some(Bytes::from_static(b"text").into())))
    );

    let framed = srv
        .block_on(framed.send(WsMessage::Ping("text".into())))
        .unwrap();
    let (item, framed) = srv.block_on(framed.into_future()).map_err(|_| ()).unwrap();
    assert_eq!(item, Some(WsFrame::Pong("text".to_string().into())));

    let framed = srv
        .block_on(framed.send(WsMessage::Close(Some(WsCloseCode::Normal.into()))))
        .unwrap();

    let (item, _framed) = srv.block_on(framed.into_future()).map_err(|_| ()).unwrap();
    assert_eq!(item, Some(WsFrame::Close(Some(WsCloseCode::Normal.into()))));
}
