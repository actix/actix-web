use actix::prelude::*;
use actix_web::{test, web, App, HttpRequest};
use actix_web_actors::*;
use bytes::Bytes;
use futures::{SinkExt, StreamExt};

struct Ws;

impl Actor for Ws {
    type Context = ws::WebsocketContext<Self>;
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for Ws {
    fn handle(
        &mut self,
        msg: Result<ws::Message, ws::ProtocolError>,
        ctx: &mut Self::Context,
    ) {
        match msg.unwrap() {
            ws::Message::Ping(msg) => ctx.pong(&msg),
            ws::Message::Text(text) => ctx.text(text),
            ws::Message::Binary(bin) => ctx.binary(bin),
            ws::Message::Close(reason) => ctx.close(reason),
            _ => (),
        }
    }
}

#[actix_rt::test]
async fn test_simple() {
    let mut srv = test::start(|| {
        App::new().service(web::resource("/").to(
            |req: HttpRequest, stream: web::Payload| async move {
                ws::start(Ws, &req, stream)
            },
        ))
    });

    // client service
    let mut framed = srv.ws().await.unwrap();
    framed.send(ws::Frame::Text("text".into())).await.unwrap();

    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, ws::Frame::Text(Bytes::from_static(b"text")));

    framed.send(ws::Frame::Binary("text".into())).await.unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, ws::Frame::Binary(Bytes::from_static(b"text").into()));

    framed.send(ws::Frame::Ping("text".into())).await.unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, ws::Frame::Pong(Bytes::copy_from_slice(b"text")));

    framed
        .send(ws::Frame::Close(Some(ws::CloseCode::Normal.into())))
        .await
        .unwrap();

    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, ws::Frame::Close(Some(ws::CloseCode::Normal.into())));
}

#[actix_rt::test]
async fn test_continuation_frames() {
    let mut srv = test::start(|| {
        App::new().service(web::resource("/").to(
            |req: HttpRequest, stream: web::Payload| async move {
                ws::start(Ws, &req, stream)
            },
        ))
    });

    // client service
    let mut framed = srv.ws().await.unwrap();
    framed
        .send(ws::Frame::Continuation(ws::Item::FirstText("first".into())))
        .await
        .unwrap();
    framed
        .send(ws::Frame::Continuation(ws::Item::Last(" text".into())))
        .await
        .unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, ws::Frame::Text(Bytes::from_static(b"first text")));

    framed
        .send(ws::Frame::Continuation(ws::Item::FirstBinary(
            "first".into(),
        )))
        .await
        .unwrap();
    framed
        .send(ws::Frame::Continuation(ws::Item::Last(" binary".into())))
        .await
        .unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, ws::Frame::Binary(Bytes::from_static(b"first binary")));

    framed
        .send(ws::Frame::Continuation(ws::Item::FirstText("first".into())))
        .await
        .unwrap();
    framed
        .send(ws::Frame::Continuation(ws::Item::Continue(
            " continuation".into(),
        )))
        .await
        .unwrap();
    framed
        .send(ws::Frame::Continuation(ws::Item::Last(" text".into())))
        .await
        .unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(
        item,
        ws::Frame::Text(Bytes::from_static(b"first continuation text"))
    );

    framed
        .send(ws::Frame::Continuation(ws::Item::FirstBinary(
            "first".into(),
        )))
        .await
        .unwrap();
    framed
        .send(ws::Frame::Continuation(ws::Item::Continue(
            " continuation".into(),
        )))
        .await
        .unwrap();
    framed
        .send(ws::Frame::Continuation(ws::Item::Last(" binary".into())))
        .await
        .unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(
        item,
        ws::Frame::Binary(Bytes::from_static(b"first continuation binary"))
    );
}
