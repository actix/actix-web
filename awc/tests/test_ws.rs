use std::io;

use actix_codec::Framed;
use actix_http::{body::BodySize, h1, ws, Error, HttpService, Request, Response};
use actix_http_test::TestServer;
use bytes::{Bytes, BytesMut};
use futures::future::ok;
use futures::{SinkExt, StreamExt};

async fn ws_service(req: ws::Frame) -> Result<ws::Message, io::Error> {
    match req {
        ws::Frame::Ping(msg) => Ok(ws::Message::Pong(msg)),
        ws::Frame::Text(text) => {
            let text = if let Some(pl) = text {
                String::from_utf8(Vec::from(pl.as_ref())).unwrap()
            } else {
                String::new()
            };
            Ok(ws::Message::Text(text))
        }
        ws::Frame::Binary(bin) => Ok(ws::Message::Binary(
            bin.map(|e| e.freeze())
                .unwrap_or_else(|| Bytes::from(""))
                .into(),
        )),
        ws::Frame::Close(reason) => Ok(ws::Message::Close(reason)),
        _ => Ok(ws::Message::Close(None)),
    }
}

#[actix_rt::test]
async fn test_simple() {
    let mut srv = TestServer::start(|| {
        HttpService::build()
            .upgrade(|(req, mut framed): (Request, Framed<_, _>)| {
                async move {
                    let res = ws::handshake_response(req.head()).finish();
                    // send handshake response
                    framed
                        .send(h1::Message::Item((res.drop_body(), BodySize::None)))
                        .await?;

                    // start websocket service
                    let framed = framed.into_framed(ws::Codec::new());
                    ws::Transport::with(framed, ws_service).await
                }
            })
            .finish(|_| ok::<_, Error>(Response::NotFound()))
    });

    // client service
    let mut framed = srv.ws().await.unwrap();
    framed
        .send(ws::Message::Text("text".to_string()))
        .await
        .unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, ws::Frame::Text(Some(BytesMut::from("text"))));

    framed
        .send(ws::Message::Binary("text".into()))
        .await
        .unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(
        item,
        ws::Frame::Binary(Some(Bytes::from_static(b"text").into()))
    );

    framed.send(ws::Message::Ping("text".into())).await.unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, ws::Frame::Pong("text".to_string().into()));

    framed
        .send(ws::Message::Close(Some(ws::CloseCode::Normal.into())))
        .await
        .unwrap();

    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, ws::Frame::Close(Some(ws::CloseCode::Normal.into())));
}
