use actix::prelude::*;
use actix_http::ws::Codec;
use actix_web::{web, App, HttpRequest};
use actix_web_actors::ws;
use bytes::Bytes;
use futures_util::{SinkExt as _, StreamExt as _};

struct Ws;

impl Actor for Ws {
    type Context = ws::WebsocketContext<Self>;
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for Ws {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(msg)) => ctx.pong(&msg),
            Ok(ws::Message::Text(text)) => ctx.text(text),
            Ok(ws::Message::Binary(bin)) => ctx.binary(bin),
            Ok(ws::Message::Close(reason)) => ctx.close(reason),
            _ => ctx.close(Some(ws::CloseCode::Error.into())),
        }
    }
}

const MAX_FRAME_SIZE: usize = 10_000;
const DEFAULT_FRAME_SIZE: usize = 10;

async fn common_test_code(mut srv: actix_test::TestServer, frame_size: usize) {
    // client service
    let mut framed = srv.ws().await.unwrap();

    framed.send(ws::Message::Text("text".into())).await.unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, ws::Frame::Text(Bytes::from_static(b"text")));

    let bytes = Bytes::from(vec![0; frame_size]);
    framed
        .send(ws::Message::Binary(bytes.clone()))
        .await
        .unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, ws::Frame::Binary(bytes));

    framed.send(ws::Message::Ping("text".into())).await.unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, ws::Frame::Pong(Bytes::copy_from_slice(b"text")));

    framed
        .send(ws::Message::Close(Some(ws::CloseCode::Normal.into())))
        .await
        .unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, ws::Frame::Close(Some(ws::CloseCode::Normal.into())));
}

#[actix_rt::test]
async fn simple_builder() {
    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").to(
            |req: HttpRequest, stream: web::Payload| async move {
                ws::WsResponseBuilder::new(Ws, &req, stream).start()
            },
        ))
    });

    common_test_code(srv, DEFAULT_FRAME_SIZE).await;
}

#[actix_rt::test]
async fn builder_with_frame_size() {
    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").to(
            |req: HttpRequest, stream: web::Payload| async move {
                ws::WsResponseBuilder::new(Ws, &req, stream)
                    .frame_size(MAX_FRAME_SIZE)
                    .start()
            },
        ))
    });

    common_test_code(srv, MAX_FRAME_SIZE).await;
}

#[actix_rt::test]
async fn builder_with_frame_size_exceeded() {
    const MAX_FRAME_SIZE: usize = 64;

    let mut srv = actix_test::start(|| {
        App::new().service(web::resource("/").to(
            |req: HttpRequest, stream: web::Payload| async move {
                ws::WsResponseBuilder::new(Ws, &req, stream)
                    .frame_size(MAX_FRAME_SIZE)
                    .start()
            },
        ))
    });

    // client service
    let mut framed = srv.ws().await.unwrap();

    // create a request with a frame size larger than expected
    let bytes = Bytes::from(vec![0; MAX_FRAME_SIZE + 1]);
    framed.send(ws::Message::Binary(bytes)).await.unwrap();

    let frame = framed.next().await.unwrap().unwrap();
    let close_reason = match frame {
        ws::Frame::Close(Some(reason)) => reason,
        _ => panic!("close frame expected"),
    };
    assert_eq!(close_reason.code, ws::CloseCode::Error);
}

#[actix_rt::test]
async fn builder_with_codec() {
    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").to(
            |req: HttpRequest, stream: web::Payload| async move {
                ws::WsResponseBuilder::new(Ws, &req, stream)
                    .codec(Codec::new())
                    .start()
            },
        ))
    });

    common_test_code(srv, DEFAULT_FRAME_SIZE).await;
}

#[actix_rt::test]
async fn builder_with_protocols() {
    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").to(
            |req: HttpRequest, stream: web::Payload| async move {
                ws::WsResponseBuilder::new(Ws, &req, stream)
                    .protocols(&["A", "B"])
                    .start()
            },
        ))
    });

    common_test_code(srv, DEFAULT_FRAME_SIZE).await;
}

#[actix_rt::test]
async fn builder_with_codec_and_frame_size() {
    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").to(
            |req: HttpRequest, stream: web::Payload| async move {
                ws::WsResponseBuilder::new(Ws, &req, stream)
                    .codec(Codec::new())
                    .frame_size(MAX_FRAME_SIZE)
                    .start()
            },
        ))
    });

    common_test_code(srv, DEFAULT_FRAME_SIZE).await;
}

#[actix_rt::test]
async fn builder_full() {
    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").to(
            |req: HttpRequest, stream: web::Payload| async move {
                ws::WsResponseBuilder::new(Ws, &req, stream)
                    .frame_size(MAX_FRAME_SIZE)
                    .codec(Codec::new())
                    .protocols(&["A", "B"])
                    .start()
            },
        ))
    });

    common_test_code(srv, MAX_FRAME_SIZE).await;
}

#[actix_rt::test]
async fn simple_start() {
    let srv = actix_test::start(|| {
        App::new().service(web::resource("/").to(
            |req: HttpRequest, stream: web::Payload| async move { ws::start(Ws, &req, stream) },
        ))
    });

    common_test_code(srv, DEFAULT_FRAME_SIZE).await;
}
