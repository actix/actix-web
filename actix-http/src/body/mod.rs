//! Traits and structures to aid consuming and writing HTTP payloads.

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

#[cfg(test)]
mod tests {
    use std::pin::Pin;

    use actix_rt::pin;
    use actix_utils::future::poll_fn;
    use bytes::{Bytes, BytesMut};
    use futures_util::stream;

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
    async fn body_stream_skips_empty_chunks() {
        let body = BodyStream::new(stream::iter(
            ["1", "", "2"]
                .iter()
                .map(|&v| Ok(Bytes::from(v)) as Result<Bytes, ()>),
        ));
        pin!(body);

        assert_eq!(
            poll_fn(|cx| body.as_mut().poll_next(cx))
                .await
                .unwrap()
                .ok(),
            Some(Bytes::from("1")),
        );
        assert_eq!(
            poll_fn(|cx| body.as_mut().poll_next(cx))
                .await
                .unwrap()
                .ok(),
            Some(Bytes::from("2")),
        );
    }

    mod sized_stream {
        use super::*;

        #[actix_rt::test]
        async fn skips_empty_chunks() {
            let body = SizedStream::new(
                2,
                stream::iter(["1", "", "2"].iter().map(|&v| Ok(Bytes::from(v)))),
            );
            pin!(body);
            assert_eq!(
                poll_fn(|cx| body.as_mut().poll_next(cx))
                    .await
                    .unwrap()
                    .ok(),
                Some(Bytes::from("1")),
            );
            assert_eq!(
                poll_fn(|cx| body.as_mut().poll_next(cx))
                    .await
                    .unwrap()
                    .ok(),
                Some(Bytes::from("2")),
            );
        }
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
}
