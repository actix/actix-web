//! [`MessageBody`] trait and foreign implementations.

use std::{
    convert::Infallible,
    error::Error as StdError,
    mem,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::{Bytes, BytesMut};
use futures_core::ready;
use pin_project_lite::pin_project;

use super::{BodySize, BoxBody};

/// An interface types that can converted to bytes and used as response bodies.
// TODO: examples
pub trait MessageBody {
    // TODO: consider this bound to only fmt::Display since the error type is not really used
    // and there is an impl for Into<Box<StdError>> on String
    type Error: Into<Box<dyn StdError>>;

    /// Body size hint.
    fn size(&self) -> BodySize;

    /// Attempt to pull out the next chunk of body bytes.
    // TODO: expand documentation
    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>>;

    /// Convert this body into `Bytes`.
    ///
    /// Body types with `BodySize::None` are allowed to return empty `Bytes`.
    #[inline]
    fn try_into_bytes(self) -> Result<Bytes, Self>
    where
        Self: Sized,
    {
        Err(self)
    }

    /// Converts this body into `BoxBody`.
    #[inline]
    fn boxed(self) -> BoxBody
    where
        Self: Sized + 'static,
    {
        BoxBody::new(self)
    }
}

mod foreign_impls {
    use super::*;

    impl MessageBody for Infallible {
        type Error = Infallible;

        fn size(&self) -> BodySize {
            match *self {}
        }

        fn poll_next(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Bytes, Self::Error>>> {
            match *self {}
        }
    }

    impl MessageBody for () {
        type Error = Infallible;

        #[inline]
        fn size(&self) -> BodySize {
            BodySize::Sized(0)
        }

        #[inline]
        fn poll_next(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Bytes, Self::Error>>> {
            Poll::Ready(None)
        }

        #[inline]
        fn try_into_bytes(self) -> Result<Bytes, Self> {
            Ok(Bytes::new())
        }
    }

    impl<B> MessageBody for Box<B>
    where
        B: MessageBody + Unpin + ?Sized,
    {
        type Error = B::Error;

        #[inline]
        fn size(&self) -> BodySize {
            self.as_ref().size()
        }

        #[inline]
        fn poll_next(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Bytes, Self::Error>>> {
            Pin::new(self.get_mut().as_mut()).poll_next(cx)
        }
    }

    impl<B> MessageBody for Pin<Box<B>>
    where
        B: MessageBody + ?Sized,
    {
        type Error = B::Error;

        #[inline]
        fn size(&self) -> BodySize {
            self.as_ref().size()
        }

        #[inline]
        fn poll_next(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Bytes, Self::Error>>> {
            self.get_mut().as_mut().poll_next(cx)
        }
    }

    impl MessageBody for &'static [u8] {
        type Error = Infallible;

        #[inline]
        fn size(&self) -> BodySize {
            BodySize::Sized(self.len() as u64)
        }

        #[inline]
        fn poll_next(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Bytes, Self::Error>>> {
            if self.is_empty() {
                Poll::Ready(None)
            } else {
                Poll::Ready(Some(Ok(Bytes::from_static(mem::take(self.get_mut())))))
            }
        }

        #[inline]
        fn try_into_bytes(self) -> Result<Bytes, Self> {
            Ok(Bytes::from_static(self))
        }
    }

    impl MessageBody for Bytes {
        type Error = Infallible;

        #[inline]
        fn size(&self) -> BodySize {
            BodySize::Sized(self.len() as u64)
        }

        #[inline]
        fn poll_next(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Bytes, Self::Error>>> {
            if self.is_empty() {
                Poll::Ready(None)
            } else {
                Poll::Ready(Some(Ok(mem::take(self.get_mut()))))
            }
        }

        #[inline]
        fn try_into_bytes(self) -> Result<Bytes, Self> {
            Ok(self)
        }
    }

    impl MessageBody for BytesMut {
        type Error = Infallible;

        #[inline]
        fn size(&self) -> BodySize {
            BodySize::Sized(self.len() as u64)
        }

        #[inline]
        fn poll_next(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Bytes, Self::Error>>> {
            if self.is_empty() {
                Poll::Ready(None)
            } else {
                Poll::Ready(Some(Ok(mem::take(self.get_mut()).freeze())))
            }
        }

        #[inline]
        fn try_into_bytes(self) -> Result<Bytes, Self> {
            Ok(self.freeze())
        }
    }

    impl MessageBody for Vec<u8> {
        type Error = Infallible;

        #[inline]
        fn size(&self) -> BodySize {
            BodySize::Sized(self.len() as u64)
        }

        #[inline]
        fn poll_next(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Bytes, Self::Error>>> {
            if self.is_empty() {
                Poll::Ready(None)
            } else {
                Poll::Ready(Some(Ok(mem::take(self.get_mut()).into())))
            }
        }

        #[inline]
        fn try_into_bytes(self) -> Result<Bytes, Self> {
            Ok(Bytes::from(self))
        }
    }

    impl MessageBody for &'static str {
        type Error = Infallible;

        #[inline]
        fn size(&self) -> BodySize {
            BodySize::Sized(self.len() as u64)
        }

        #[inline]
        fn poll_next(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Bytes, Self::Error>>> {
            if self.is_empty() {
                Poll::Ready(None)
            } else {
                let string = mem::take(self.get_mut());
                let bytes = Bytes::from_static(string.as_bytes());
                Poll::Ready(Some(Ok(bytes)))
            }
        }

        #[inline]
        fn try_into_bytes(self) -> Result<Bytes, Self> {
            Ok(Bytes::from_static(self.as_bytes()))
        }
    }

    impl MessageBody for String {
        type Error = Infallible;

        #[inline]
        fn size(&self) -> BodySize {
            BodySize::Sized(self.len() as u64)
        }

        #[inline]
        fn poll_next(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Bytes, Self::Error>>> {
            if self.is_empty() {
                Poll::Ready(None)
            } else {
                let string = mem::take(self.get_mut());
                Poll::Ready(Some(Ok(Bytes::from(string))))
            }
        }

        #[inline]
        fn try_into_bytes(self) -> Result<Bytes, Self> {
            Ok(Bytes::from(self))
        }
    }

    impl MessageBody for bytestring::ByteString {
        type Error = Infallible;

        #[inline]
        fn size(&self) -> BodySize {
            BodySize::Sized(self.len() as u64)
        }

        #[inline]
        fn poll_next(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Bytes, Self::Error>>> {
            let string = mem::take(self.get_mut());
            Poll::Ready(Some(Ok(string.into_bytes())))
        }

        #[inline]
        fn try_into_bytes(self) -> Result<Bytes, Self> {
            Ok(self.into_bytes())
        }
    }
}

pin_project! {
    pub(crate) struct MessageBodyMapErr<B, F> {
        #[pin]
        body: B,
        mapper: Option<F>,
    }
}

impl<B, F, E> MessageBodyMapErr<B, F>
where
    B: MessageBody,
    F: FnOnce(B::Error) -> E,
{
    pub(crate) fn new(body: B, mapper: F) -> Self {
        Self {
            body,
            mapper: Some(mapper),
        }
    }
}

impl<B, F, E> MessageBody for MessageBodyMapErr<B, F>
where
    B: MessageBody,
    F: FnOnce(B::Error) -> E,
    E: Into<Box<dyn StdError>>,
{
    type Error = E;

    #[inline]
    fn size(&self) -> BodySize {
        self.body.size()
    }

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        let this = self.as_mut().project();

        match ready!(this.body.poll_next(cx)) {
            Some(Err(err)) => {
                let f = self.as_mut().project().mapper.take().unwrap();
                let mapped_err = (f)(err);
                Poll::Ready(Some(Err(mapped_err)))
            }
            Some(Ok(val)) => Poll::Ready(Some(Ok(val))),
            None => Poll::Ready(None),
        }
    }

    #[inline]
    fn try_into_bytes(self) -> Result<Bytes, Self> {
        let Self { body, mapper } = self;
        body.try_into_bytes().map_err(|body| Self { body, mapper })
    }
}

#[cfg(test)]
mod tests {
    use actix_rt::pin;
    use actix_utils::future::poll_fn;
    use bytes::{Bytes, BytesMut};

    use super::*;
    use crate::body::{self, EitherBody};

    macro_rules! assert_poll_next {
        ($pin:expr, $exp:expr) => {
            assert_eq!(
                poll_fn(|cx| $pin.as_mut().poll_next(cx))
                    .await
                    .unwrap() // unwrap option
                    .unwrap(), // unwrap result
                $exp
            );
        };
    }

    macro_rules! assert_poll_next_none {
        ($pin:expr) => {
            assert!(poll_fn(|cx| $pin.as_mut().poll_next(cx)).await.is_none());
        };
    }

    #[actix_rt::test]
    async fn boxing_equivalence() {
        assert_eq!(().size(), BodySize::Sized(0));
        assert_eq!(().size(), Box::new(()).size());
        assert_eq!(().size(), Box::pin(()).size());

        let pl = Box::new(());
        pin!(pl);
        assert_poll_next_none!(pl);

        let mut pl = Box::pin(());
        assert_poll_next_none!(pl);
    }

    #[actix_rt::test]
    async fn test_unit() {
        let pl = ();
        assert_eq!(pl.size(), BodySize::Sized(0));
        pin!(pl);
        assert_poll_next_none!(pl);
    }

    #[actix_rt::test]
    async fn test_static_str() {
        assert_eq!("".size(), BodySize::Sized(0));
        assert_eq!("test".size(), BodySize::Sized(4));

        let pl = "test";
        pin!(pl);
        assert_poll_next!(pl, Bytes::from("test"));
    }

    #[actix_rt::test]
    async fn test_static_bytes() {
        assert_eq!(b"".as_ref().size(), BodySize::Sized(0));
        assert_eq!(b"test".as_ref().size(), BodySize::Sized(4));

        let pl = b"test".as_ref();
        pin!(pl);
        assert_poll_next!(pl, Bytes::from("test"));
    }

    #[actix_rt::test]
    async fn test_vec() {
        assert_eq!(vec![0; 0].size(), BodySize::Sized(0));
        assert_eq!(Vec::from("test").size(), BodySize::Sized(4));

        let pl = Vec::from("test");
        pin!(pl);
        assert_poll_next!(pl, Bytes::from("test"));
    }

    #[actix_rt::test]
    async fn test_bytes() {
        assert_eq!(Bytes::new().size(), BodySize::Sized(0));
        assert_eq!(Bytes::from_static(b"test").size(), BodySize::Sized(4));

        let pl = Bytes::from_static(b"test");
        pin!(pl);
        assert_poll_next!(pl, Bytes::from("test"));
    }

    #[actix_rt::test]
    async fn test_bytes_mut() {
        assert_eq!(BytesMut::new().size(), BodySize::Sized(0));
        assert_eq!(BytesMut::from(b"test".as_ref()).size(), BodySize::Sized(4));

        let pl = BytesMut::from("test");
        pin!(pl);
        assert_poll_next!(pl, Bytes::from("test"));
    }

    #[actix_rt::test]
    async fn test_string() {
        assert_eq!(String::new().size(), BodySize::Sized(0));
        assert_eq!("test".to_owned().size(), BodySize::Sized(4));

        let pl = "test".to_owned();
        pin!(pl);
        assert_poll_next!(pl, Bytes::from("test"));
    }

    #[actix_rt::test]
    async fn complete_body_combinators() {
        let body = Bytes::from_static(b"test");
        let body = BoxBody::new(body);
        let body = EitherBody::<_, ()>::left(body);
        let body = EitherBody::<(), _>::right(body);
        // Do not support try_into_bytes:
        // let body = Box::new(body);
        // let body = Box::pin(body);

        assert_eq!(body.try_into_bytes().unwrap(), Bytes::from("test"));
    }

    #[actix_rt::test]
    async fn complete_body_combinators_poll() {
        let body = Bytes::from_static(b"test");
        let body = BoxBody::new(body);
        let body = EitherBody::<_, ()>::left(body);
        let body = EitherBody::<(), _>::right(body);
        let mut body = body;

        assert_eq!(body.size(), BodySize::Sized(4));
        assert_poll_next!(Pin::new(&mut body), Bytes::from("test"));
        assert_poll_next_none!(Pin::new(&mut body));
    }

    #[actix_rt::test]
    async fn none_body_combinators() {
        fn none_body() -> BoxBody {
            let body = body::None;
            let body = BoxBody::new(body);
            let body = EitherBody::<_, ()>::left(body);
            let body = EitherBody::<(), _>::right(body);
            body.boxed()
        }

        assert_eq!(none_body().size(), BodySize::None);
        assert_eq!(none_body().try_into_bytes().unwrap(), Bytes::new());
        assert_poll_next_none!(Pin::new(&mut none_body()));
    }

    // down-casting used to be done with a method on MessageBody trait
    // test is kept to demonstrate equivalence of Any trait
    #[actix_rt::test]
    async fn test_body_casting() {
        let mut body = String::from("hello cast");
        // let mut resp_body: &mut dyn MessageBody<Error = Error> = &mut body;
        let resp_body: &mut dyn std::any::Any = &mut body;
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
