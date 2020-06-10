// Regression test for #/1321

/*
use futures::task::{noop_waker, Context};
use futures::stream::once;
use actix_http::body::{MessageBody, BodyStream};
use bytes::Bytes;

Disable weird poll until actix-web is based on actix-http 2.0.0

#[test]
fn weird_poll() {
    let (sender, receiver) = futures::channel::oneshot::channel();
    let mut body_stream = Ok(BodyStream::new(once(async {
        let x = Box::new(0);
        let y = &x;
        receiver.await.unwrap();
        let _z = **y;
        Ok::<_, ()>(Bytes::new())
    })));

    let waker = noop_waker();
    let mut context = Context::from_waker(&waker);

    let _ = body_stream.as_mut().unwrap().poll_next(&mut context);
    sender.send(()).unwrap();
    let _ = std::mem::replace(&mut body_stream, Err([0; 32])).unwrap().poll_next(&mut context);
}

*/
