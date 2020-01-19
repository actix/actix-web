use actix_codec::{AsyncRead, AsyncWrite};
use actix_http::{body, http::StatusCode, ws, Error, HttpService, Response};
use actix_http_test::test_server;
use actix_service::{pipeline_factory, IntoServiceFactory, ServiceFactory};
use actix_utils::framed::Dispatcher;
use bytes::Bytes;
use futures::{future, SinkExt, StreamExt};

use actix_framed::{FramedApp, FramedRequest, FramedRoute, SendError, VerifyWebSockets};

async fn ws_service<T: AsyncRead + AsyncWrite>(
    req: FramedRequest<T>,
) -> Result<(), Error> {
    let (req, mut framed, _) = req.into_parts();
    let res = ws::handshake(req.head()).unwrap().message_body(());

    framed
        .send((res, body::BodySize::None).into())
        .await
        .unwrap();
    Dispatcher::new(framed.into_framed(ws::Codec::new()), service)
        .await
        .unwrap();

    Ok(())
}

async fn service(msg: ws::Frame) -> Result<ws::Message, Error> {
    let msg = match msg {
        ws::Frame::Ping(msg) => ws::Message::Pong(msg),
        ws::Frame::Text(text) => {
            ws::Message::Text(String::from_utf8_lossy(&text).to_string())
        }
        ws::Frame::Binary(bin) => ws::Message::Binary(bin),
        ws::Frame::Close(reason) => ws::Message::Close(reason),
        _ => panic!(),
    };
    Ok(msg)
}

#[actix_rt::test]
async fn test_simple() {
    let mut srv = test_server(|| {
        HttpService::build()
            .upgrade(
                FramedApp::new().service(FramedRoute::get("/index.html").to(ws_service)),
            )
            .finish(|_| future::ok::<_, Error>(Response::NotFound()))
            .tcp()
    });

    assert!(srv.ws_at("/test").await.is_err());

    // client service
    let mut framed = srv.ws_at("/index.html").await.unwrap();
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
        ws::Frame::Binary(Bytes::from_static(b"text"))
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

    let (item, _) = framed.into_future().await;
    assert_eq!(
        item.unwrap().unwrap(),
        ws::Frame::Close(Some(ws::CloseCode::Normal.into()))
    );
}

#[actix_rt::test]
async fn test_service() {
    let mut srv = test_server(|| {
        pipeline_factory(actix_http::h1::OneRequest::new().map_err(|_| ())).and_then(
            pipeline_factory(
                pipeline_factory(VerifyWebSockets::default())
                    .then(SendError::default())
                    .map_err(|_| ()),
            )
            .and_then(
                FramedApp::new()
                    .service(FramedRoute::get("/index.html").to(ws_service))
                    .into_factory()
                    .map_err(|_| ()),
            ),
        )
    });

    // non ws request
    let res = srv.get("/index.html").send().await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // not found
    assert!(srv.ws_at("/test").await.is_err());

    // client service
    let mut framed = srv.ws_at("/index.html").await.unwrap();
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
        ws::Frame::Binary(Bytes::from_static(b"text"))
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

    let (item, _) = framed.into_future().await;
    assert_eq!(
        item.unwrap().unwrap(),
        ws::Frame::Close(Some(ws::CloseCode::Normal.into()))
    );
}
