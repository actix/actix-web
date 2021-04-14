//! Traits and structures to aid consuming and writing HTTP payloads.

use std::task::Poll;

use actix_rt::pin;
use actix_utils::future::poll_fn;
use bytes::{Bytes, BytesMut};
use futures_core::ready;

#[allow(clippy::module_inception)]
mod body;
mod body_stream;
mod message_body;
mod response_body;
mod size;
mod sized_stream;

pub use self::body::Body;
pub use self::body_stream::BodyStream;
pub use self::message_body::MessageBody;
pub use self::response_body::ResponseBody;
pub use self::size::BodySize;
pub use self::sized_stream::SizedStream;

/// Collects the body produced by a `MessageBody` implementation into `Bytes`.
///
/// Any errors produced by the body stream are returned immediately.
///
/// # Examples
/// ```
/// use actix_http::body::{Body, to_bytes};
/// use bytes::Bytes;
///
/// # async fn test_to_bytes() {
/// let body = Body::Empty;
/// let bytes = to_bytes(body).await.unwrap();
/// assert!(bytes.is_empty());
///
/// let body = Body::Bytes(Bytes::from_static(b"123"));
/// let bytes = to_bytes(body).await.unwrap();
/// assert_eq!(bytes, b"123"[..]);
/// # }
/// ```
pub async fn to_bytes(body: impl MessageBody) -> Result<Bytes, crate::Error> {
    let cap = match body.size() {
        BodySize::None | BodySize::Empty | BodySize::Sized(0) => return Ok(Bytes::new()),
        BodySize::Sized(size) => size as usize,
        BodySize::Stream => 32_768,
    };

    let mut buf = BytesMut::with_capacity(cap);

    pin!(body);

    poll_fn(|cx| loop {
        let body = body.as_mut();

        match ready!(body.poll_next(cx)) {
            Some(Ok(bytes)) => buf.extend_from_slice(&*bytes),
            None => return Poll::Ready(Ok(())),
            Some(Err(err)) => return Poll::Ready(Err(err)),
        }
    })
    .await?;

    Ok(buf.freeze())
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;

    use actix_rt::pin;
    use actix_utils::future::poll_fn;
    use bytes::{Bytes, BytesMut};

    use super::*;

    impl Body {
        pub(crate) fn get_ref(&self) -> &[u8] {
            match *self {
                Body::Bytes(ref bin) => &bin,
                _ => panic!(),
            }
        }
    }

    impl ResponseBody<Body> {
        pub(crate) fn get_ref(&self) -> &[u8] {
            match *self {
                ResponseBody::Body(ref b) => b.get_ref(),
                ResponseBody::Other(ref b) => b.get_ref(),
            }
        }
    }

    #[actix_rt::test]
    async fn test_static_str() {
        assert_eq!(Body::from("").size(), BodySize::Sized(0));
        assert_eq!(Body::from("test").size(), BodySize::Sized(4));
        assert_eq!(Body::from("test").get_ref(), b"test");

        assert_eq!("test".size(), BodySize::Sized(4));
        assert_eq!(
            poll_fn(|cx| Pin::new(&mut "test").poll_next(cx))
                .await
                .unwrap()
                .ok(),
            Some(Bytes::from("test"))
        );
    }

    #[actix_rt::test]
    async fn test_static_bytes() {
        assert_eq!(Body::from(b"test".as_ref()).size(), BodySize::Sized(4));
        assert_eq!(Body::from(b"test".as_ref()).get_ref(), b"test");
        assert_eq!(
            Body::from_slice(b"test".as_ref()).size(),
            BodySize::Sized(4)
        );
        assert_eq!(Body::from_slice(b"test".as_ref()).get_ref(), b"test");
        let sb = Bytes::from(&b"test"[..]);
        pin!(sb);

        assert_eq!(sb.size(), BodySize::Sized(4));
        assert_eq!(
            poll_fn(|cx| sb.as_mut().poll_next(cx)).await.unwrap().ok(),
            Some(Bytes::from("test"))
        );
    }

    #[actix_rt::test]
    async fn test_vec() {
        assert_eq!(Body::from(Vec::from("test")).size(), BodySize::Sized(4));
        assert_eq!(Body::from(Vec::from("test")).get_ref(), b"test");
        let test_vec = Vec::from("test");
        pin!(test_vec);

        assert_eq!(test_vec.size(), BodySize::Sized(4));
        assert_eq!(
            poll_fn(|cx| test_vec.as_mut().poll_next(cx))
                .await
                .unwrap()
                .ok(),
            Some(Bytes::from("test"))
        );
    }

    #[actix_rt::test]
    async fn test_bytes() {
        let b = Bytes::from("test");
        assert_eq!(Body::from(b.clone()).size(), BodySize::Sized(4));
        assert_eq!(Body::from(b.clone()).get_ref(), b"test");
        pin!(b);

        assert_eq!(b.size(), BodySize::Sized(4));
        assert_eq!(
            poll_fn(|cx| b.as_mut().poll_next(cx)).await.unwrap().ok(),
            Some(Bytes::from("test"))
        );
    }

    #[actix_rt::test]
    async fn test_bytes_mut() {
        let b = BytesMut::from("test");
        assert_eq!(Body::from(b.clone()).size(), BodySize::Sized(4));
        assert_eq!(Body::from(b.clone()).get_ref(), b"test");
        pin!(b);

        assert_eq!(b.size(), BodySize::Sized(4));
        assert_eq!(
            poll_fn(|cx| b.as_mut().poll_next(cx)).await.unwrap().ok(),
            Some(Bytes::from("test"))
        );
    }

    #[actix_rt::test]
    async fn test_string() {
        let b = "test".to_owned();
        assert_eq!(Body::from(b.clone()).size(), BodySize::Sized(4));
        assert_eq!(Body::from(b.clone()).get_ref(), b"test");
        assert_eq!(Body::from(&b).size(), BodySize::Sized(4));
        assert_eq!(Body::from(&b).get_ref(), b"test");
        pin!(b);

        assert_eq!(b.size(), BodySize::Sized(4));
        assert_eq!(
            poll_fn(|cx| b.as_mut().poll_next(cx)).await.unwrap().ok(),
            Some(Bytes::from("test"))
        );
    }

    #[actix_rt::test]
    async fn test_unit() {
        assert_eq!(().size(), BodySize::Empty);
        assert!(poll_fn(|cx| Pin::new(&mut ()).poll_next(cx))
            .await
            .is_none());
    }

    #[actix_rt::test]
    async fn test_box() {
        let val = Box::new(());
        pin!(val);
        assert_eq!(val.size(), BodySize::Empty);
        assert!(poll_fn(|cx| val.as_mut().poll_next(cx)).await.is_none());
    }

    #[actix_rt::test]
    async fn test_body_eq() {
        assert!(
            Body::Bytes(Bytes::from_static(b"1"))
                == Body::Bytes(Bytes::from_static(b"1"))
        );
        assert!(Body::Bytes(Bytes::from_static(b"1")) != Body::None);
    }

    #[actix_rt::test]
    async fn test_body_debug() {
        assert!(format!("{:?}", Body::None).contains("Body::None"));
        assert!(format!("{:?}", Body::Empty).contains("Body::Empty"));
        assert!(format!("{:?}", Body::Bytes(Bytes::from_static(b"1"))).contains('1'));
    }

    #[actix_rt::test]
    async fn test_serde_json() {
        use serde_json::{json, Value};
        assert_eq!(
            Body::from(serde_json::to_vec(&Value::String("test".to_owned())).unwrap())
                .size(),
            BodySize::Sized(6)
        );
        assert_eq!(
            Body::from(serde_json::to_vec(&json!({"test-key":"test-value"})).unwrap())
                .size(),
            BodySize::Sized(25)
        );
    }

    #[actix_rt::test]
    async fn test_body_casting() {
        let mut body = String::from("hello cast");
        let resp_body: &mut dyn MessageBody = &mut body;
        let body = resp_body.downcast_ref::<String>().unwrap();
        assert_eq!(body, "hello cast");
        let body = &mut resp_body.downcast_mut::<String>().unwrap();
        body.push('!');
        let body = resp_body.downcast_ref::<String>().unwrap();
        assert_eq!(body, "hello cast!");
        let not_body = resp_body.downcast_ref::<()>();
        assert!(not_body.is_none());
    }

    #[actix_rt::test]
    async fn test_to_bytes() {
        let body = Body::Empty;
        let bytes = to_bytes(body).await.unwrap();
        assert!(bytes.is_empty());

        let body = Body::Bytes(Bytes::from_static(b"123"));
        let bytes = to_bytes(body).await.unwrap();
        assert_eq!(bytes, b"123"[..]);
    }
}
