use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_http::{body, h1, ws, Error, HttpService, Request, Response};
use actix_http_test::TestServer;
use actix_utils::framed::FramedTransport;
use bytes::{Bytes, BytesMut};
use futures::future;
use futures::{SinkExt, StreamExt};

async fn ws_service<T: AsyncRead + AsyncWrite + Unpin>(
    (req, mut framed): (Request, Framed<T, h1::Codec>),
) -> Result<(), Error> {
    let res = ws::handshake(req.head()).unwrap().message_body(());

    framed
        .send((res, body::BodySize::None).into())
        .await
        .unwrap();

    FramedTransport::new(framed.into_framed(ws::Codec::new()), service)
        .await
        .map_err(|_| panic!())
}

async fn service(msg: ws::Frame) -> Result<ws::Message, Error> {
    let msg = match msg {
        ws::Frame::Ping(msg) => ws::Message::Pong(msg),
        ws::Frame::Text(text) => {
            ws::Message::Text(String::from_utf8_lossy(&text.unwrap()).to_string())
        }
        ws::Frame::Binary(bin) => ws::Message::Binary(bin.unwrap().freeze()),
        ws::Frame::Close(reason) => ws::Message::Close(reason),
        _ => panic!(),
    };
    Ok(msg)
}

#[actix_rt::test]
async fn test_simple() {
    let mut srv = TestServer::start(|| {
        HttpService::build()
            .upgrade(actix_service::service_fn(ws_service))
            .finish(|_| future::ok::<_, ()>(Response::NotFound()))
            .tcp()
    });

    // client service
    let mut framed = srv.ws().await.unwrap();
    framed
        .send(ws::Message::Text("text".to_string()))
        .await
        .unwrap();
    let (item, mut framed) = framed.into_future().await;
    assert_eq!(
        item.unwrap().unwrap(),
        ws::Frame::Text(Some(BytesMut::from("text")))
    );

    framed
        .send(ws::Message::Binary("text".into()))
        .await
        .unwrap();
    let (item, mut framed) = framed.into_future().await;
    assert_eq!(
        item.unwrap().unwrap(),
        ws::Frame::Binary(Some(Bytes::from_static(b"text").into()))
    );

    framed.send(ws::Message::Ping("text".into())).await.unwrap();
    let (item, mut framed) = framed.into_future().await;
    assert_eq!(
        item.unwrap().unwrap(),
        ws::Frame::Pong("text".to_string().into())
    );

    framed
        .send(ws::Message::Close(Some(ws::CloseCode::Normal.into())))
        .await
        .unwrap();

    let (item, _framed) = framed.into_future().await;
    assert_eq!(
        item.unwrap().unwrap(),
        ws::Frame::Close(Some(ws::CloseCode::Normal.into()))
    );
}
