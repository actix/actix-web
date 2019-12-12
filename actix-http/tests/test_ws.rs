use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_http::{body, h1, ws, Error, HttpService, Request, Response};
use actix_http_test::TestServer;
use actix_utils::framed::Dispatcher;
use bytes::Bytes;
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

    Dispatcher::new(framed.into_framed(ws::Codec::new()), service)
        .await
        .map_err(|_| panic!())
}

async fn service(msg: ws::Frame) -> Result<ws::Message, Error> {
    let msg = match msg {
        ws::Frame::Ping(msg) => ws::Message::Pong(msg),
        ws::Frame::Text(text) => {
            ws::Message::Text(String::from_utf8_lossy(&text).to_string())
        }
        ws::Frame::Binary(bin) => ws::Message::Binary(bin),
        ws::Frame::Continuation(item) => ws::Message::Continuation(item),
        ws::Frame::Close(reason) => ws::Message::Close(reason),
        _ => panic!(),
    };
    Ok(msg)
}

#[actix_rt::test]
async fn test_simple() {
    let mut srv = TestServer::start(|| {
        HttpService::build()
            .upgrade(actix_service::fn_service(ws_service))
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
        ws::Frame::Text(Bytes::from_static(b"text"))
    );

    framed
        .send(ws::Message::Binary("text".into()))
        .await
        .unwrap();
    let (item, mut framed) = framed.into_future().await;
    assert_eq!(
        item.unwrap().unwrap(),
        ws::Frame::Binary(Bytes::from_static(&b"text"[..]))
    );

    framed.send(ws::Message::Ping("text".into())).await.unwrap();
    let (item, mut framed) = framed.into_future().await;
    assert_eq!(
        item.unwrap().unwrap(),
        ws::Frame::Pong("text".to_string().into())
    );

    framed
        .send(ws::Message::Continuation(ws::Item::FirstText(
            "text".into(),
        )))
        .await
        .unwrap();
    let (item, mut framed) = framed.into_future().await;
    assert_eq!(
        item.unwrap().unwrap(),
        ws::Frame::Continuation(ws::Item::FirstText(Bytes::from_static(b"text")))
    );

    assert!(framed
        .send(ws::Message::Continuation(ws::Item::FirstText(
            "text".into()
        )))
        .await
        .is_err());
    assert!(framed
        .send(ws::Message::Continuation(ws::Item::FirstBinary(
            "text".into()
        )))
        .await
        .is_err());

    framed
        .send(ws::Message::Continuation(ws::Item::Continue("text".into())))
        .await
        .unwrap();
    let (item, mut framed) = framed.into_future().await;
    assert_eq!(
        item.unwrap().unwrap(),
        ws::Frame::Continuation(ws::Item::Continue(Bytes::from_static(b"text")))
    );

    framed
        .send(ws::Message::Continuation(ws::Item::Last("text".into())))
        .await
        .unwrap();
    let (item, mut framed) = framed.into_future().await;
    assert_eq!(
        item.unwrap().unwrap(),
        ws::Frame::Continuation(ws::Item::Last(Bytes::from_static(b"text")))
    );

    assert!(framed
        .send(ws::Message::Continuation(ws::Item::Continue("text".into())))
        .await
        .is_err());

    assert!(framed
        .send(ws::Message::Continuation(ws::Item::Last("text".into())))
        .await
        .is_err());

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
