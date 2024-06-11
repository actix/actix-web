use std::{
    cell::Cell,
    convert::Infallible,
    task::{Context, Poll},
};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_http::{
    body::{BodySize, BoxBody},
    h1,
    ws::{self, CloseCode, Frame, Item, Message},
    Error, HttpService, Request, Response,
};
use actix_http_test::test_server;
use actix_service::{fn_factory, Service};
use bytes::Bytes;
use derive_more::{Display, Error, From};
use futures_core::future::LocalBoxFuture;
use futures_util::{SinkExt as _, StreamExt as _};

#[derive(Clone)]
struct WsService(Cell<bool>);

impl WsService {
    fn new() -> Self {
        WsService(Cell::new(false))
    }

    fn set_polled(&self) {
        self.0.set(true);
    }

    fn was_polled(&self) -> bool {
        self.0.get()
    }
}

#[derive(Debug, Display, Error, From)]
enum WsServiceError {
    #[display(fmt = "HTTP error")]
    Http(actix_http::Error),

    #[display(fmt = "WS handshake error")]
    Ws(actix_http::ws::HandshakeError),

    #[display(fmt = "I/O error")]
    Io(std::io::Error),

    #[display(fmt = "dispatcher error")]
    Dispatcher,
}

impl From<WsServiceError> for Response<BoxBody> {
    fn from(err: WsServiceError) -> Self {
        match err {
            WsServiceError::Http(err) => err.into(),
            WsServiceError::Ws(err) => err.into(),
            WsServiceError::Io(_err) => unreachable!(),
            WsServiceError::Dispatcher => {
                Response::internal_server_error().set_body(BoxBody::new(format!("{}", err)))
            }
        }
    }
}

impl<T> Service<(Request, Framed<T, h1::Codec>)> for WsService
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Response = ();
    type Error = WsServiceError;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.set_polled();
        Poll::Ready(Ok(()))
    }

    fn call(&self, (req, mut framed): (Request, Framed<T, h1::Codec>)) -> Self::Future {
        assert!(self.was_polled());

        Box::pin(async move {
            let res = ws::handshake(req.head())?.message_body(())?;

            framed.send((res, BodySize::None).into()).await?;

            let framed = framed.replace_codec(ws::Codec::new());

            ws::Dispatcher::with(framed, service)
                .await
                .map_err(|_| WsServiceError::Dispatcher)?;

            Ok(())
        })
    }
}

async fn service(msg: Frame) -> Result<Message, Error> {
    let msg = match msg {
        Frame::Ping(msg) => Message::Pong(msg),
        Frame::Text(text) => Message::Text(String::from_utf8_lossy(&text).into_owned().into()),
        Frame::Binary(bin) => Message::Binary(bin),
        Frame::Continuation(item) => Message::Continuation(item),
        Frame::Close(reason) => Message::Close(reason),
        _ => return Err(ws::ProtocolError::BadOpCode.into()),
    };

    Ok(msg)
}

#[actix_rt::test]
async fn simple() {
    let mut srv = test_server(|| {
        HttpService::build()
            .upgrade(fn_factory(|| async {
                Ok::<_, Infallible>(WsService::new())
            }))
            .finish(|_| async { Ok::<_, Infallible>(Response::not_found()) })
            .tcp()
    })
    .await;

    // client service
    let mut framed = srv.ws().await.unwrap();
    framed.send(Message::Text("text".into())).await.unwrap();

    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, Frame::Text(Bytes::from_static(b"text")));

    framed.send(Message::Binary("text".into())).await.unwrap();

    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, Frame::Binary(Bytes::from_static(&b"text"[..])));

    framed.send(Message::Ping("text".into())).await.unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, Frame::Pong("text".to_string().into()));

    framed
        .send(Message::Continuation(Item::FirstText("text".into())))
        .await
        .unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(
        item,
        Frame::Continuation(Item::FirstText(Bytes::from_static(b"text")))
    );

    assert!(framed
        .send(Message::Continuation(Item::FirstText("text".into())))
        .await
        .is_err());
    assert!(framed
        .send(Message::Continuation(Item::FirstBinary("text".into())))
        .await
        .is_err());

    framed
        .send(Message::Continuation(Item::Continue("text".into())))
        .await
        .unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(
        item,
        Frame::Continuation(Item::Continue(Bytes::from_static(b"text")))
    );

    framed
        .send(Message::Continuation(Item::Last("text".into())))
        .await
        .unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(
        item,
        Frame::Continuation(Item::Last(Bytes::from_static(b"text")))
    );

    assert!(framed
        .send(Message::Continuation(Item::Continue("text".into())))
        .await
        .is_err());

    assert!(framed
        .send(Message::Continuation(Item::Last("text".into())))
        .await
        .is_err());

    framed
        .send(Message::Close(Some(CloseCode::Normal.into())))
        .await
        .unwrap();

    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, Frame::Close(Some(CloseCode::Normal.into())));
}
