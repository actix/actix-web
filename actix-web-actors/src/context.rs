use std::collections::VecDeque;

use actix::dev::{
    AsyncContextParts, ContextFut, ContextParts, Envelope, Mailbox, ToEnvelope,
};
use actix::fut::ActorFuture;
use actix::{
    Actor, ActorContext, ActorState, Addr, AsyncContext, Handler, Message, SpawnHandle,
};
use actix_web::error::{Error, ErrorInternalServerError};
use bytes::Bytes;
use futures::sync::oneshot::Sender;
use futures::{Async, Future, Poll, Stream};

/// Execution context for http actors
pub struct HttpContext<A>
where
    A: Actor<Context = HttpContext<A>>,
{
    inner: ContextParts<A>,
    stream: VecDeque<Option<Bytes>>,
}

impl<A> ActorContext for HttpContext<A>
where
    A: Actor<Context = Self>,
{
    fn stop(&mut self) {
        self.inner.stop();
    }
    fn terminate(&mut self) {
        self.inner.terminate()
    }
    fn state(&self) -> ActorState {
        self.inner.state()
    }
}

impl<A> AsyncContext<A> for HttpContext<A>
where
    A: Actor<Context = Self>,
{
    #[inline]
    fn spawn<F>(&mut self, fut: F) -> SpawnHandle
    where
        F: ActorFuture<Item = (), Error = (), Actor = A> + 'static,
    {
        self.inner.spawn(fut)
    }

    #[inline]
    fn wait<F>(&mut self, fut: F)
    where
        F: ActorFuture<Item = (), Error = (), Actor = A> + 'static,
    {
        self.inner.wait(fut)
    }

    #[doc(hidden)]
    #[inline]
    fn waiting(&self) -> bool {
        self.inner.waiting()
            || self.inner.state() == ActorState::Stopping
            || self.inner.state() == ActorState::Stopped
    }

    #[inline]
    fn cancel_future(&mut self, handle: SpawnHandle) -> bool {
        self.inner.cancel_future(handle)
    }

    #[inline]
    fn address(&self) -> Addr<A> {
        self.inner.address()
    }
}

impl<A> HttpContext<A>
where
    A: Actor<Context = Self>,
{
    #[inline]
    /// Create a new HTTP Context from a request and an actor
    pub fn create(actor: A) -> impl Stream<Item = Bytes, Error = Error> {
        let mb = Mailbox::default();
        let ctx = HttpContext {
            inner: ContextParts::new(mb.sender_producer()),
            stream: VecDeque::new(),
        };
        HttpContextFut::new(ctx, actor, mb)
    }

    /// Create a new HTTP Context
    pub fn with_factory<F>(f: F) -> impl Stream<Item = Bytes, Error = Error>
    where
        F: FnOnce(&mut Self) -> A + 'static,
    {
        let mb = Mailbox::default();
        let mut ctx = HttpContext {
            inner: ContextParts::new(mb.sender_producer()),
            stream: VecDeque::new(),
        };

        let act = f(&mut ctx);
        HttpContextFut::new(ctx, act, mb)
    }
}

impl<A> HttpContext<A>
where
    A: Actor<Context = Self>,
{
    /// Write payload
    #[inline]
    pub fn write(&mut self, data: Bytes) {
        self.stream.push_back(Some(data));
    }

    /// Indicate end of streaming payload. Also this method calls `Self::close`.
    #[inline]
    pub fn write_eof(&mut self) {
        self.stream.push_back(None);
    }

    /// Handle of the running future
    ///
    /// SpawnHandle is the handle returned by `AsyncContext::spawn()` method.
    pub fn handle(&self) -> SpawnHandle {
        self.inner.curr_handle()
    }
}

impl<A> AsyncContextParts<A> for HttpContext<A>
where
    A: Actor<Context = Self>,
{
    fn parts(&mut self) -> &mut ContextParts<A> {
        &mut self.inner
    }
}

struct HttpContextFut<A>
where
    A: Actor<Context = HttpContext<A>>,
{
    fut: ContextFut<A, HttpContext<A>>,
}

impl<A> HttpContextFut<A>
where
    A: Actor<Context = HttpContext<A>>,
{
    fn new(ctx: HttpContext<A>, act: A, mailbox: Mailbox<A>) -> Self {
        let fut = ContextFut::new(ctx, act, mailbox);
        HttpContextFut { fut }
    }
}

impl<A> Stream for HttpContextFut<A>
where
    A: Actor<Context = HttpContext<A>>,
{
    type Item = Bytes;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Bytes>, Error> {
        if self.fut.alive() {
            match self.fut.poll() {
                Ok(Async::NotReady) | Ok(Async::Ready(())) => (),
                Err(_) => return Err(ErrorInternalServerError("error")),
            }
        }

        // frames
        if let Some(data) = self.fut.ctx().stream.pop_front() {
            Ok(Async::Ready(data))
        } else if self.fut.alive() {
            Ok(Async::NotReady)
        } else {
            Ok(Async::Ready(None))
        }
    }
}

impl<A, M> ToEnvelope<A, M> for HttpContext<A>
where
    A: Actor<Context = HttpContext<A>> + Handler<M>,
    M: Message + Send + 'static,
    M::Result: Send,
{
    fn pack(msg: M, tx: Option<Sender<M::Result>>) -> Envelope<A> {
        Envelope::new(msg, tx)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use actix::Actor;
    use actix_web::http::StatusCode;
    use actix_web::test::{block_on, call_service, init_service, TestRequest};
    use actix_web::{web, App, HttpResponse};
    use bytes::{Bytes, BytesMut};

    use super::*;

    struct MyActor {
        count: usize,
    }

    impl Actor for MyActor {
        type Context = HttpContext<Self>;

        fn started(&mut self, ctx: &mut Self::Context) {
            ctx.run_later(Duration::from_millis(100), |slf, ctx| slf.write(ctx));
        }
    }

    impl MyActor {
        fn write(&mut self, ctx: &mut HttpContext<Self>) {
            self.count += 1;
            if self.count > 3 {
                ctx.write_eof()
            } else {
                ctx.write(Bytes::from(format!("LINE-{}", self.count).as_bytes()));
                ctx.run_later(Duration::from_millis(100), |slf, ctx| slf.write(ctx));
            }
        }
    }

    #[test]
    fn test_default_resource() {
        let mut srv =
            init_service(App::new().service(web::resource("/test").to(|| {
                HttpResponse::Ok().streaming(HttpContext::create(MyActor { count: 0 }))
            })));

        let req = TestRequest::with_uri("/test").to_request();
        let mut resp = call_service(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::OK);

        let body = block_on(resp.take_body().fold(
            BytesMut::new(),
            move |mut body, chunk| {
                body.extend_from_slice(&chunk);
                Ok::<_, Error>(body)
            },
        ))
        .unwrap();
        assert_eq!(body.freeze(), Bytes::from_static(b"LINE-1LINE-2LINE-3"));
    }
}
