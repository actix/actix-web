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

    /// Returns true if entire body bytes chunk is obtainable in one call to `poll_next`.
    ///
    /// This method's implementation should agree with [`take_complete_body`] and should always be
    /// checked before taking the body.
    ///
    /// The default implementation returns `false.
    ///
    /// [`take_complete_body`]: MessageBody::take_complete_body
    fn is_complete_body(&self) -> bool {
        false
    }

    /// Returns the complete chunk of body bytes.
    ///
    /// Implementors of this method should note the following:
    /// - It is acceptable to skip the omit checks of [`is_complete_body`]. The responsibility of
    ///   performing this check is delegated to the caller.
    /// - If the result of [`is_complete_body`] is conditional, that condition should be given
    ///   equivalent attention here.
    /// - A second call call to [`take_complete_body`] should return an empty `Bytes` or panic.
    /// - A call to [`poll_next`] after calling [`take_complete_body`] should return `None` unless
    ///   the chunk is guaranteed to be empty.
    ///
    /// The default implementation panics unconditionally, indicating a control flow bug in the
    /// calling code.
    ///
    /// # Panics
    /// With a correct implementation, panics if called without first checking [`is_complete_body`].
    ///
    /// [`is_complete_body`]: MessageBody::is_complete_body
    /// [`take_complete_body`]: MessageBody::take_complete_body
    /// [`poll_next`]: MessageBody::poll_next
    fn take_complete_body(&mut self) -> Bytes {
        assert!(
            self.is_complete_body(),
            "type ({}) allows taking complete body but did not provide an implementation \
            of `take_complete_body`",
            std::any::type_name::<Self>()
        );

        unimplemented!(
            "type ({}) does not allow taking complete body; caller should make sure to \
            check `is_complete_body` first",
            std::any::type_name::<Self>()
        );
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

        fn is_complete_body(&self) -> bool {
            true
        }

        fn take_complete_body(&mut self) -> Bytes {
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
        fn is_complete_body(&self) -> bool {
            true
        }

        #[inline]
        fn take_complete_body(&mut self) -> Bytes {
            Bytes::new()
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

        #[inline]
        fn is_complete_body(&self) -> bool {
            self.as_ref().is_complete_body()
        }

        #[inline]
        fn take_complete_body(&mut self) -> Bytes {
            self.as_mut().take_complete_body()
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

        #[inline]
        fn is_complete_body(&self) -> bool {
            self.as_ref().is_complete_body()
        }

        #[inline]
        fn take_complete_body(&mut self) -> Bytes {
            debug_assert!(
                self.is_complete_body(),
                "inner type \"{}\" does not allow taking complete body; caller should make sure to \
                call `is_complete_body` first",
                std::any::type_name::<B>(),
            );

            // we do not have DerefMut access to call take_complete_body directly but since
            // is_complete_body is true we should expect the entire bytes chunk in one poll_next

            let waker = futures_task::noop_waker();
            let mut cx = Context::from_waker(&waker);

            match self.as_mut().poll_next(&mut cx) {
                Poll::Ready(Some(Ok(data))) => data,
                _ => {
                    panic!(
                        "inner type \"{}\" indicated it allows taking complete body but failed to \
                        return Bytes when polled",
                        std::any::type_name::<B>()
                    );
                }
            }
        }
    }

    impl MessageBody for &'static [u8] {
        type Error = Infallible;

        #[inline]
        fn size(&self) -> BodySize {
            BodySize::Sized(self.len() as u64)
        }

        fn poll_next(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Bytes, Self::Error>>> {
            if self.is_empty() {
                Poll::Ready(None)
            } else {
                Poll::Ready(Some(Ok(self.take_complete_body())))
            }
        }

        #[inline]
        fn is_complete_body(&self) -> bool {
            true
        }

        #[inline]
        fn take_complete_body(&mut self) -> Bytes {
            Bytes::from_static(mem::take(self))
        }
    }

    impl MessageBody for Bytes {
        type Error = Infallible;

        #[inline]
        fn size(&self) -> BodySize {
            BodySize::Sized(self.len() as u64)
        }

        fn poll_next(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Bytes, Self::Error>>> {
            if self.is_empty() {
                Poll::Ready(None)
            } else {
                Poll::Ready(Some(Ok(self.take_complete_body())))
            }
        }

        #[inline]
        fn is_complete_body(&self) -> bool {
            true
        }

        #[inline]
        fn take_complete_body(&mut self) -> Bytes {
            mem::take(self)
        }
    }

    impl MessageBody for BytesMut {
        type Error = Infallible;

        #[inline]
        fn size(&self) -> BodySize {
            BodySize::Sized(self.len() as u64)
        }

        fn poll_next(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Bytes, Self::Error>>> {
            if self.is_empty() {
                Poll::Ready(None)
            } else {
                Poll::Ready(Some(Ok(self.take_complete_body())))
            }
        }

        #[inline]
        fn is_complete_body(&self) -> bool {
            true
        }

        #[inline]
        fn take_complete_body(&mut self) -> Bytes {
            mem::take(self).freeze()
        }
    }

    impl MessageBody for Vec<u8> {
        type Error = Infallible;

        #[inline]
        fn size(&self) -> BodySize {
            BodySize::Sized(self.len() as u64)
        }

        fn poll_next(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Bytes, Self::Error>>> {
            if self.is_empty() {
                Poll::Ready(None)
            } else {
                Poll::Ready(Some(Ok(self.take_complete_body())))
            }
        }

        #[inline]
        fn is_complete_body(&self) -> bool {
            true
        }

        #[inline]
        fn take_complete_body(&mut self) -> Bytes {
            Bytes::from(mem::take(self))
        }
    }

    impl MessageBody for &'static str {
        type Error = Infallible;

        #[inline]
        fn size(&self) -> BodySize {
            BodySize::Sized(self.len() as u64)
        }

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
        fn is_complete_body(&self) -> bool {
            true
        }

        #[inline]
        fn take_complete_body(&mut self) -> Bytes {
            Bytes::from_static(mem::take(self).as_bytes())
        }
    }

    impl MessageBody for String {
        type Error = Infallible;

        #[inline]
        fn size(&self) -> BodySize {
            BodySize::Sized(self.len() as u64)
        }

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
        fn is_complete_body(&self) -> bool {
            true
        }

        #[inline]
        fn take_complete_body(&mut self) -> Bytes {
            Bytes::from(mem::take(self))
        }
    }

    impl MessageBody for bytestring::ByteString {
        type Error = Infallible;

        #[inline]
        fn size(&self) -> BodySize {
            BodySize::Sized(self.len() as u64)
        }

        fn poll_next(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Bytes, Self::Error>>> {
            let string = mem::take(self.get_mut());
            Poll::Ready(Some(Ok(string.into_bytes())))
        }

        #[inline]
        fn is_complete_body(&self) -> bool {
            true
        }

        #[inline]
        fn take_complete_body(&mut self) -> Bytes {
            mem::take(self).into_bytes()
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
    fn is_complete_body(&self) -> bool {
        self.body.is_complete_body()
    }

    #[inline]
    fn take_complete_body(&mut self) -> Bytes {
        self.body.take_complete_body()
    }
}

#[cfg(test)]
mod tests {
    use actix_rt::pin;
    use actix_utils::future::poll_fn;
    use bytes::{Bytes, BytesMut};

    use super::*;

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

    #[test]
    fn take_string() {
        let mut data = "test".repeat(2);
        let data_bytes = Bytes::from(data.clone());
        assert!(data.is_complete_body());
        assert_eq!(data.take_complete_body(), data_bytes);

        let mut big_data = "test".repeat(64 * 1024);
        let data_bytes = Bytes::from(big_data.clone());
        assert!(big_data.is_complete_body());
        assert_eq!(big_data.take_complete_body(), data_bytes);
    }

    #[test]
    fn take_boxed_equivalence() {
        let mut data = Bytes::from_static(b"test");
        assert!(data.is_complete_body());
        assert_eq!(data.take_complete_body(), b"test".as_ref());

        let mut data = Box::new(Bytes::from_static(b"test"));
        assert!(data.is_complete_body());
        assert_eq!(data.take_complete_body(), b"test".as_ref());

        let mut data = Box::pin(Bytes::from_static(b"test"));
        assert!(data.is_complete_body());
        assert_eq!(data.take_complete_body(), b"test".as_ref());
    }

    #[test]
    fn take_policy() {
        let mut data = Bytes::from_static(b"test");
        // first call returns chunk
        assert_eq!(data.take_complete_body(), b"test".as_ref());
        // second call returns empty
        assert_eq!(data.take_complete_body(), b"".as_ref());

        let waker = futures_task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut data = Bytes::from_static(b"test");
        // take returns whole chunk
        assert_eq!(data.take_complete_body(), b"test".as_ref());
        // subsequent poll_next returns None
        assert_eq!(Pin::new(&mut data).poll_next(&mut cx), Poll::Ready(None));
    }

    #[test]
    fn complete_body_combinators() {
        use crate::body::{BoxBody, EitherBody};

        let body = Bytes::from_static(b"test");
        let body = BoxBody::new(body);
        let body = EitherBody::<_, ()>::left(body);
        let body = EitherBody::<(), _>::right(body);
        let body = Box::new(body);
        let body = Box::pin(body);
        let mut body = body;

        assert!(body.is_complete_body());
        assert_eq!(body.take_complete_body(), b"test".as_ref());

        // subsequent poll_next returns None
        let waker = futures_task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        assert!(Pin::new(&mut body).poll_next(&mut cx).map_err(drop) == Poll::Ready(None));
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
