//! Regression test for https://github.com/actix/actix-web/issues/1321

// use actix_http::body::{BodyStream, MessageBody};
// use bytes::Bytes;
// use futures_channel::oneshot;
// use futures_util::{
//     stream::once,
//     task::{noop_waker, Context},
// };

// #[test]
// fn weird_poll() {
//     let (sender, receiver) = oneshot::channel();
//     let mut body_stream = Ok(BodyStream::new(once(async {
//         let x = Box::new(0);
//         let y = &x;
//         receiver.await.unwrap();
//         let _z = **y;
//         Ok::<_, ()>(Bytes::new())
//     })));

//     let waker = noop_waker();
//     let mut cx = Context::from_waker(&waker);

//     let _ = body_stream.as_mut().unwrap().poll_next(&mut cx);
//     sender.send(()).unwrap();
//     let _ = std::mem::replace(&mut body_stream, Err([0; 32]))
//         .unwrap()
//         .poll_next(&mut cx);
// }
