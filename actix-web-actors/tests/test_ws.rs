use actix::prelude::*;
use actix_web::{
    http::{header, StatusCode},
    web, App, HttpRequest, HttpResponse,
};
use actix_web_actors::*;
use bytes::Bytes;
use futures_util::{SinkExt as _, StreamExt as _};

struct Ws;

impl Actor for Ws {
    type Context = ws::WebsocketContext<Self>;
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for Ws {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg.unwrap() {
            ws::Message::Ping(msg) => ctx.pong(&msg),
            ws::Message::Text(text) => ctx.text(text),
            ws::Message::Binary(bin) => ctx.binary(bin),
            ws::Message::Close(reason) => ctx.close(reason),
            _ => {}
        }
    }
}

#[actix_rt::test]
async fn test_simple() {
    let mut srv = actix_test::start(|| {
        App::new().service(web::resource("/").to(
            |req: HttpRequest, stream: web::Payload| async move { ws::start(Ws, &req, stream) },
        ))
    });

    // client service
    let mut framed = srv.ws().await.unwrap();
    framed.send(ws::Message::Text("text".into())).await.unwrap();

    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, ws::Frame::Text(Bytes::from_static(b"text")));

    framed
        .send(ws::Message::Binary("text".into()))
        .await
        .unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, ws::Frame::Binary(Bytes::from_static(b"text")));

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
async fn test_with_credentials() {
    let mut srv = actix_test::start(|| {
        App::new().service(web::resource("/").to(
            |req: HttpRequest, stream: web::Payload| async move {
                if req.headers().contains_key("Authorization") {
                    ws::start(Ws, &req, stream)
                } else {
                    Ok(HttpResponse::new(StatusCode::UNAUTHORIZED))
                }
            },
        ))
    });

    // client service without credentials
    match srv.ws().await {
        Ok(_) => panic!("WebSocket client without credentials should panic"),
        Err(awc::error::WsClientError::InvalidResponseStatus(status)) => {
            assert_eq!(status, StatusCode::UNAUTHORIZED)
        }
        Err(e) => panic!("Invalid error from WebSocket client: {}", e),
    }

    let headers = srv.client_headers().unwrap();
    headers.insert(
        header::AUTHORIZATION,
        header::HeaderValue::from_static("Bearer Something"),
    );

    // client service with credentials
    let client = srv.ws();

    let mut framed = client.await.unwrap();

    framed.send(ws::Message::Text("text".into())).await.unwrap();

    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, ws::Frame::Text(Bytes::from_static(b"text")));

    framed
        .send(ws::Message::Close(Some(ws::CloseCode::Normal.into())))
        .await
        .unwrap();

    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, ws::Frame::Close(Some(ws::CloseCode::Normal.into())));
}
