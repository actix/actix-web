use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::{fmt, mem};

use bytes::{Bytes, BytesMut};
use futures_core::Stream;
use futures_util::ready;
use pin_project::pin_project;

use crate::error::Error;

#[derive(Debug, PartialEq, Copy, Clone)]
/// Body size hint
pub enum BodySize {
    None,
    Empty,
    Sized(u64),
    Stream,
}

impl BodySize {
    pub fn is_eof(&self) -> bool {
        matches!(self, BodySize::None | BodySize::Empty | BodySize::Sized(0))
    }
}

/// Type that provides this trait can be streamed to a peer.
pub trait MessageBody {
    fn size(&self) -> BodySize;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>>;

    downcast_get_type_id!();
}

downcast!(MessageBody);

impl MessageBody for () {
    fn size(&self) -> BodySize {
        BodySize::Empty
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        Poll::Ready(None)
    }
}

impl<T: MessageBody + Unpin> MessageBody for Box<T> {
    fn size(&self) -> BodySize {
        self.as_ref().size()
    }

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        Pin::new(self.get_mut().as_mut()).poll_next(cx)
    }
}

#[pin_project(project = ResponseBodyProj)]
pub enum ResponseBody<B> {
    Body(#[pin] B),
    Other(#[pin] Body),
}

impl ResponseBody<Body> {
    pub fn into_body<B>(self) -> ResponseBody<B> {
        match self {
            ResponseBody::Body(b) => ResponseBody::Other(b),
            ResponseBody::Other(b) => ResponseBody::Other(b),
        }
    }
}

impl<B> ResponseBody<B> {
    pub fn take_body(&mut self) -> ResponseBody<B> {
        std::mem::replace(self, ResponseBody::Other(Body::None))
    }
}

impl<B: MessageBody> ResponseBody<B> {
    pub fn as_ref(&self) -> Option<&B> {
        if let ResponseBody::Body(ref b) = self {
            Some(b)
        } else {
            None
        }
    }
}

impl<B: MessageBody> MessageBody for ResponseBody<B> {
    fn size(&self) -> BodySize {
        match self {
            ResponseBody::Body(ref body) => body.size(),
            ResponseBody::Other(ref body) => body.size(),
        }
    }

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        match self.project() {
            ResponseBodyProj::Body(body) => body.poll_next(cx),
            ResponseBodyProj::Other(body) => body.poll_next(cx),
        }
    }
}

impl<B: MessageBody> Stream for ResponseBody<B> {
    type Item = Result<Bytes, Error>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        match self.project() {
            ResponseBodyProj::Body(body) => body.poll_next(cx),
            ResponseBodyProj::Other(body) => body.poll_next(cx),
        }
    }
}

#[pin_project(project = BodyProj)]
/// Represents various types of http message body.
pub enum Body {
    /// Empty response. `Content-Length` header is not set.
    None,
    /// Zero sized response body. `Content-Length` header is set to `0`.
    Empty,
    /// Specific response body.
    Bytes(Bytes),
    /// Generic message body.
    Message(Box<dyn MessageBody + Unpin>),
}

impl Body {
    /// Create body from slice (copy)
    pub fn from_slice(s: &[u8]) -> Body {
        Body::Bytes(Bytes::copy_from_slice(s))
    }

    /// Create body from generic message body.
    pub fn from_message<B: MessageBody + Unpin + 'static>(body: B) -> Body {
        Body::Message(Box::new(body))
    }
}

impl MessageBody for Body {
    fn size(&self) -> BodySize {
        match self {
            Body::None => BodySize::None,
            Body::Empty => BodySize::Empty,
            Body::Bytes(ref bin) => BodySize::Sized(bin.len() as u64),
            Body::Message(ref body) => body.size(),
        }
    }

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        match self.project() {
            BodyProj::None => Poll::Ready(None),
            BodyProj::Empty => Poll::Ready(None),
            BodyProj::Bytes(ref mut bin) => {
                let len = bin.len();
                if len == 0 {
                    Poll::Ready(None)
                } else {
                    Poll::Ready(Some(Ok(mem::take(bin))))
                }
            }
            BodyProj::Message(ref mut body) => Pin::new(body.as_mut()).poll_next(cx),
        }
    }
}

impl PartialEq for Body {
    fn eq(&self, other: &Body) -> bool {
        match *self {
            Body::None => matches!(*other, Body::None),
            Body::Empty => matches!(*other, Body::Empty),
            Body::Bytes(ref b) => match *other {
                Body::Bytes(ref b2) => b == b2,
                _ => false,
            },
            Body::Message(_) => false,
        }
    }
}

impl fmt::Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Body::None => write!(f, "Body::None"),
            Body::Empty => write!(f, "Body::Empty"),
            Body::Bytes(ref b) => write!(f, "Body::Bytes({:?})", b),
            Body::Message(_) => write!(f, "Body::Message(_)"),
        }
    }
}

impl From<&'static str> for Body {
    fn from(s: &'static str) -> Body {
        Body::Bytes(Bytes::from_static(s.as_ref()))
    }
}

impl From<&'static [u8]> for Body {
    fn from(s: &'static [u8]) -> Body {
        Body::Bytes(Bytes::from_static(s))
    }
}

impl From<Vec<u8>> for Body {
    fn from(vec: Vec<u8>) -> Body {
        Body::Bytes(Bytes::from(vec))
    }
}

impl From<String> for Body {
    fn from(s: String) -> Body {
        s.into_bytes().into()
    }
}

impl<'a> From<&'a String> for Body {
    fn from(s: &'a String) -> Body {
        Body::Bytes(Bytes::copy_from_slice(AsRef::<[u8]>::as_ref(&s)))
    }
}

impl From<Bytes> for Body {
    fn from(s: Bytes) -> Body {
        Body::Bytes(s)
    }
}

impl From<BytesMut> for Body {
    fn from(s: BytesMut) -> Body {
        Body::Bytes(s.freeze())
    }
}

impl From<serde_json::Value> for Body {
    fn from(v: serde_json::Value) -> Body {
        Body::Bytes(v.to_string().into())
    }
}

impl<S> From<SizedStream<S>> for Body
where
    S: Stream<Item = Result<Bytes, Error>> + Unpin + 'static,
{
    fn from(s: SizedStream<S>) -> Body {
        Body::from_message(s)
    }
}

impl<S, E> From<BodyStream<S, E>> for Body
where
    S: Stream<Item = Result<Bytes, E>> + Unpin + 'static,
    E: Into<Error> + 'static,
{
    fn from(s: BodyStream<S, E>) -> Body {
        Body::from_message(s)
    }
}

impl MessageBody for Bytes {
    fn size(&self) -> BodySize {
        BodySize::Sized(self.len() as u64)
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        if self.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Ready(Some(Ok(mem::take(self.get_mut()))))
        }
    }
}

impl MessageBody for BytesMut {
    fn size(&self) -> BodySize {
        BodySize::Sized(self.len() as u64)
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        if self.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Ready(Some(Ok(mem::take(self.get_mut()).freeze())))
        }
    }
}

impl MessageBody for &'static str {
    fn size(&self) -> BodySize {
        BodySize::Sized(self.len() as u64)
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        if self.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Ready(Some(Ok(Bytes::from_static(
                mem::take(self.get_mut()).as_ref(),
            ))))
        }
    }
}

impl MessageBody for Vec<u8> {
    fn size(&self) -> BodySize {
        BodySize::Sized(self.len() as u64)
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        if self.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Ready(Some(Ok(Bytes::from(mem::take(self.get_mut())))))
        }
    }
}

impl MessageBody for String {
    fn size(&self) -> BodySize {
        BodySize::Sized(self.len() as u64)
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        if self.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Ready(Some(Ok(Bytes::from(
                mem::take(self.get_mut()).into_bytes(),
            ))))
        }
    }
}

/// Type represent streaming body.
/// Response does not contain `content-length` header and appropriate transfer encoding is used.
#[pin_project]
pub struct BodyStream<S: Unpin, E> {
    #[pin]
    stream: S,
    _t: PhantomData<E>,
}

impl<S, E> BodyStream<S, E>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    E: Into<Error>,
{
    pub fn new(stream: S) -> Self {
        BodyStream {
            stream,
            _t: PhantomData,
        }
    }
}

impl<S, E> MessageBody for BodyStream<S, E>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    E: Into<Error>,
{
    fn size(&self) -> BodySize {
        BodySize::Stream
    }

    /// Attempts to pull out the next value of the underlying [`Stream`].
    ///
    /// Empty values are skipped to prevent [`BodyStream`]'s transmission being
    /// ended on a zero-length chunk, but rather proceed until the underlying
    /// [`Stream`] ends.
    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        let mut stream = self.project().stream;
        loop {
            let stream = stream.as_mut();
            return Poll::Ready(match ready!(stream.poll_next(cx)) {
                Some(Ok(ref bytes)) if bytes.is_empty() => continue,
                opt => opt.map(|res| res.map_err(Into::into)),
            });
        }
    }
}

/// Type represent streaming body. This body implementation should be used
/// if total size of stream is known. Data get sent as is without using transfer encoding.
#[pin_project]
pub struct SizedStream<S: Unpin> {
    size: u64,
    #[pin]
    stream: S,
}

impl<S> SizedStream<S>
where
    S: Stream<Item = Result<Bytes, Error>> + Unpin,
{
    pub fn new(size: u64, stream: S) -> Self {
        SizedStream { size, stream }
    }
}

impl<S> MessageBody for SizedStream<S>
where
    S: Stream<Item = Result<Bytes, Error>> + Unpin,
{
    fn size(&self) -> BodySize {
        BodySize::Sized(self.size as u64)
    }

    /// Attempts to pull out the next value of the underlying [`Stream`].
    ///
    /// Empty values are skipped to prevent [`SizedStream`]'s transmission being
    /// ended on a zero-length chunk, but rather proceed until the underlying
    /// [`Stream`] ends.
    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        let mut stream: Pin<&mut S> = self.project().stream;
        loop {
            let stream = stream.as_mut();
            return Poll::Ready(match ready!(stream.poll_next(cx)) {
                Some(Ok(ref bytes)) if bytes.is_empty() => continue,
                val => val,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::future::poll_fn;
    use futures_util::pin_mut;
    use futures_util::stream;

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
        pin_mut!(sb);

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
        pin_mut!(test_vec);

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
        pin_mut!(b);

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
        pin_mut!(b);

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
        pin_mut!(b);

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
        pin_mut!(val);
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
        use serde_json::json;
        assert_eq!(
            Body::from(serde_json::Value::String("test".into())).size(),
            BodySize::Sized(6)
        );
        assert_eq!(
            Body::from(json!({"test-key":"test-value"})).size(),
            BodySize::Sized(25)
        );
    }

    mod body_stream {
        use super::*;
        //use futures::task::noop_waker;
        //use futures::stream::once;

        #[actix_rt::test]
        async fn skips_empty_chunks() {
            let body = BodyStream::new(stream::iter(
                ["1", "", "2"]
                    .iter()
                    .map(|&v| Ok(Bytes::from(v)) as Result<Bytes, ()>),
            ));
            pin_mut!(body);

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

        /* Now it does not compile as it should
        #[actix_rt::test]
        async fn move_pinned_pointer() {
            let (sender, receiver) = futures::channel::oneshot::channel();
            let mut body_stream = Ok(BodyStream::new(once(async {
                let x = Box::new(0i32);
                let y = &x;
                receiver.await.unwrap();
                let _z = **y;
                Ok::<_, ()>(Bytes::new())
            })));

            let waker = noop_waker();
            let mut context = Context::from_waker(&waker);
            pin_mut!(body_stream);

            let _ = body_stream.as_mut().unwrap().poll_next(&mut context);
            sender.send(()).unwrap();
            let _ = std::mem::replace(&mut body_stream, Err([0; 32])).unwrap().poll_next(&mut context);
        }*/
    }

    mod sized_stream {
        use super::*;

        #[actix_rt::test]
        async fn skips_empty_chunks() {
            let body = SizedStream::new(
                2,
                stream::iter(["1", "", "2"].iter().map(|&v| Ok(Bytes::from(v)))),
            );
            pin_mut!(body);
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
