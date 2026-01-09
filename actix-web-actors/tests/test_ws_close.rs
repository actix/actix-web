use actix::prelude::*;
use actix_web::{web, App, HttpRequest};
use actix_web_actors::ws;
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::sync::mpsc::Sender;

struct Ws {
    finished: Sender<()>,
}

impl Actor for Ws {
    type Context = ws::WebsocketContext<Self>;
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for Ws {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Close(reason)) => ctx.close(reason),
            _ => ctx.close(Some(ws::CloseCode::Normal.into())),
        }
    }

    fn finished(&mut self, _ctx: &mut Self::Context) {
        _ = self.finished.try_send(()).unwrap();
    }
}

#[actix_rt::test]
async fn close_initiated_by_client() {
    let (tx, mut finished) = tokio::sync::mpsc::channel(1);
    let mut srv = actix_test::start(move || {
        let tx = tx.clone();
        App::new().service(web::resource("{anything:.*}").to(
            move |req: HttpRequest, stream: web::Payload| {
                let tx: Sender<()> = tx.clone();
                async move { ws::WsResponseBuilder::new(Ws { finished: tx }, &req, stream).start() }
            },
        ))
    });

    let mut framed = srv.ws().await.unwrap();

    framed
        .send(ws::Message::Close(Some(ws::CloseCode::Normal.into())))
        .await
        .unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, ws::Frame::Close(Some(ws::CloseCode::Normal.into())));

    let nothing = actix_rt::time::timeout(std::time::Duration::from_secs(1), framed.next()).await;
    assert_eq!(true, nothing.is_ok());
    assert_eq!(true, nothing.unwrap().is_none());

    let finished =
        actix_rt::time::timeout(std::time::Duration::from_secs(1), finished.recv()).await;
    assert_eq!(true, finished.is_ok());
    assert_eq!(Some(()), finished.unwrap());
}

#[actix_rt::test]
async fn close_initiated_by_server() {
    let (tx, mut finished) = tokio::sync::mpsc::channel(1);
    let mut srv = actix_test::start(move || {
        let tx = tx.clone();
        App::new().service(web::resource("{anything:.*}").to(
            move |req: HttpRequest, stream: web::Payload| {
                let tx: Sender<()> = tx.clone();
                async move { ws::WsResponseBuilder::new(Ws { finished: tx }, &req, stream).start() }
            },
        ))
    });

    let mut framed = srv.ws().await.unwrap();

    framed
        .send(ws::Message::Text("I'll initiate close by server".into()))
        .await
        .unwrap();

    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, ws::Frame::Close(Some(ws::CloseCode::Normal.into())));

    framed
        .send(ws::Message::Close(Some(ws::CloseCode::Normal.into())))
        .await
        .unwrap();

    let nothing = actix_rt::time::timeout(std::time::Duration::from_secs(1), framed.next()).await;
    assert_eq!(true, nothing.is_ok());
    assert_eq!(true, nothing.unwrap().is_none());

    let finished =
        actix_rt::time::timeout(std::time::Duration::from_secs(1), finished.recv()).await;
    assert_eq!(true, finished.is_ok());
    assert_eq!(Some(()), finished.unwrap());
}
