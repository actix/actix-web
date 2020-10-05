use std::cell::Cell;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_http::{body, h1, ws, Error, HttpService, Request, Response};
use actix_http_test::test_server;
use actix_service::{fn_factory, Service};
use actix_utils::dispatcher::Dispatcher;
use bytes::Bytes;
use futures_util::future;
use futures_util::task::{Context, Poll};
use futures_util::{SinkExt, StreamExt};

struct WsService<T>(Arc<Mutex<(PhantomData<T>, Cell<bool>)>>);

impl<T> WsService<T> {
    fn new() -> Self {
        WsService(Arc::new(Mutex::new((PhantomData, Cell::new(false)))))
    }

    fn set_polled(&mut self) {
        *self.0.lock().unwrap().1.get_mut() = true;
    }

    fn was_polled(&self) -> bool {
        self.0.lock().unwrap().1.get()
    }
}

impl<T> Clone for WsService<T> {
    fn clone(&self) -> Self {
        WsService(self.0.clone())
    }
}

impl<T> Service for WsService<T>
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Request = (Request, Framed<T, h1::Codec>);
    type Response = ();
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<(), Error>>>>;

    fn poll_ready(&mut self, _ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.set_polled();
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, (req, mut framed): Self::Request) -> Self::Future {
        let fut = async move {
            let res = ws::handshake(req.head()).unwrap().message_body(());

            framed
                .send((res, body::BodySize::None).into())
                .await
                .unwrap();

            Dispatcher::new(framed.replace_codec(ws::Codec::new()), service)
                .await
                .map_err(|_| panic!())
        };

        Box::pin(fut)
    }
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
    let ws_service = WsService::new();
    let mut srv = test_server({
        let ws_service = ws_service.clone();
        move || {
            let ws_service = ws_service.clone();
            HttpService::build()
                .upgrade(fn_factory(move || future::ok::<_, ()>(ws_service.clone())))
                .finish(|_| future::ok::<_, ()>(Response::NotFound()))
                .tcp()
        }
    })
    .await;

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

    assert!(ws_service.was_polled());
}
