use futures::{try_ready, Async, Future, Poll};

use super::{IntoNewService, NewService, Service};
use crate::cell::Cell;

/// Service for the `and_then` combinator, chaining a computation onto the end
/// of another service which completes successfully.
///
/// This is created by the `ServiceExt::and_then` method.
pub struct AndThen<A, B> {
    a: A,
    b: Cell<B>,
}

impl<A, B> AndThen<A, B> {
    /// Create new `AndThen` combinator
    pub fn new<Request>(a: A, b: B) -> Self
    where
        A: Service<Request>,
        B: Service<A::Response, Error = A::Error>,
    {
        Self { a, b: Cell::new(b) }
    }
}

impl<A, B> Clone for AndThen<A, B>
where
    A: Clone,
{
    fn clone(&self) -> Self {
        AndThen {
            a: self.a.clone(),
            b: self.b.clone(),
        }
    }
}

impl<A, B, Request> Service<Request> for AndThen<A, B>
where
    A: Service<Request>,
    B: Service<A::Response, Error = A::Error>,
{
    type Response = B::Response;
    type Error = A::Error;
    type Future = AndThenFuture<A, B, Request>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        try_ready!(self.a.poll_ready());
        self.b.get_mut().poll_ready()
    }

    fn call(&mut self, req: Request) -> Self::Future {
        AndThenFuture::new(self.a.call(req), self.b.clone())
    }
}

pub struct AndThenFuture<A, B, Request>
where
    A: Service<Request>,
    B: Service<A::Response, Error = A::Error>,
{
    b: Cell<B>,
    fut_b: Option<B::Future>,
    fut_a: Option<A::Future>,
}

impl<A, B, Request> AndThenFuture<A, B, Request>
where
    A: Service<Request>,
    B: Service<A::Response, Error = A::Error>,
{
    fn new(a: A::Future, b: Cell<B>) -> Self {
        AndThenFuture {
            b,
            fut_a: Some(a),
            fut_b: None,
        }
    }
}

impl<A, B, Request> Future for AndThenFuture<A, B, Request>
where
    A: Service<Request>,
    B: Service<A::Response, Error = A::Error>,
{
    type Item = B::Response;
    type Error = A::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut_b {
            return fut.poll();
        }

        match self.fut_a.as_mut().expect("Bug in actix-service").poll() {
            Ok(Async::Ready(resp)) => {
                let _ = self.fut_a.take();
                self.fut_b = Some(self.b.get_mut().call(resp));
                self.poll()
            }
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(err) => Err(err),
        }
    }
}

/// `AndThenNewService` new service combinator
pub struct AndThenNewService<A, B> {
    a: A,
    b: B,
}

impl<A, B> AndThenNewService<A, B> {
    /// Create new `AndThen` combinator
    pub fn new<Request, F: IntoNewService<B, A::Response>>(a: A, f: F) -> Self
    where
        A: NewService<Request>,
        B: NewService<A::Response, Error = A::Error, InitError = A::InitError>,
    {
        Self {
            a,
            b: f.into_new_service(),
        }
    }
}

impl<A, B, Request> NewService<Request> for AndThenNewService<A, B>
where
    A: NewService<Request>,
    B: NewService<A::Response, Error = A::Error, InitError = A::InitError>,
{
    type Response = B::Response;
    type Error = A::Error;
    type Service = AndThen<A::Service, B::Service>;

    type InitError = A::InitError;
    type Future = AndThenNewServiceFuture<A, B, Request>;

    fn new_service(&self) -> Self::Future {
        AndThenNewServiceFuture::new(self.a.new_service(), self.b.new_service())
    }
}

impl<A, B> Clone for AndThenNewService<A, B>
where
    A: Clone,
    B: Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            b: self.b.clone(),
        }
    }
}

pub struct AndThenNewServiceFuture<A, B, Request>
where
    A: NewService<Request>,
    B: NewService<A::Response>,
{
    fut_b: B::Future,
    fut_a: A::Future,
    a: Option<A::Service>,
    b: Option<B::Service>,
}

impl<A, B, Request> AndThenNewServiceFuture<A, B, Request>
where
    A: NewService<Request>,
    B: NewService<A::Response>,
{
    fn new(fut_a: A::Future, fut_b: B::Future) -> Self {
        AndThenNewServiceFuture {
            fut_a,
            fut_b,
            a: None,
            b: None,
        }
    }
}

impl<A, B, Request> Future for AndThenNewServiceFuture<A, B, Request>
where
    A: NewService<Request>,
    B: NewService<A::Response, Error = A::Error, InitError = A::InitError>,
{
    type Item = AndThen<A::Service, B::Service>;
    type Error = A::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if self.a.is_none() {
            if let Async::Ready(service) = self.fut_a.poll()? {
                self.a = Some(service);
            }
        }

        if self.b.is_none() {
            if let Async::Ready(service) = self.fut_b.poll()? {
                self.b = Some(service);
            }
        }

        if self.a.is_some() && self.b.is_some() {
            Ok(Async::Ready(AndThen::new(
                self.a.take().unwrap(),
                self.b.take().unwrap(),
            )))
        } else {
            Ok(Async::NotReady)
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::future::{ok, FutureResult};
    use futures::{Async, Poll};
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use crate::{NewService, Service};

    struct Srv1(Rc<Cell<usize>>);
    impl Service<&'static str> for Srv1 {
        type Response = &'static str;
        type Error = ();
        type Future = FutureResult<Self::Response, ()>;

        fn poll_ready(&mut self) -> Poll<(), Self::Error> {
            self.0.set(self.0.get() + 1);
            Ok(Async::Ready(()))
        }

        fn call(&mut self, req: &'static str) -> Self::Future {
            ok(req)
        }
    }

    #[derive(Clone)]
    struct Srv2(Rc<Cell<usize>>);

    impl Service<&'static str> for Srv2 {
        type Response = (&'static str, &'static str);
        type Error = ();
        type Future = FutureResult<Self::Response, ()>;

        fn poll_ready(&mut self) -> Poll<(), Self::Error> {
            self.0.set(self.0.get() + 1);
            Ok(Async::Ready(()))
        }

        fn call(&mut self, req: &'static str) -> Self::Future {
            ok((req, "srv2"))
        }
    }

    #[test]
    fn test_poll_ready() {
        let cnt = Rc::new(Cell::new(0));
        let mut srv = Srv1(cnt.clone()).and_then(Srv2(cnt.clone()));
        let res = srv.poll_ready();
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), Async::Ready(()));
        assert_eq!(cnt.get(), 2);
    }

    #[test]
    fn test_call() {
        let cnt = Rc::new(Cell::new(0));
        let mut srv = Srv1(cnt.clone()).and_then(Srv2(cnt));
        let res = srv.call("srv1").poll();
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), Async::Ready(("srv1", "srv2")));
    }

    #[test]
    fn test_new_service() {
        let cnt = Rc::new(Cell::new(0));
        let cnt2 = cnt.clone();
        let blank = move || Ok::<_, ()>(Srv1(cnt2.clone()));
        let new_srv = blank
            .into_new_service()
            .and_then(move || Ok(Srv2(cnt.clone())));
        if let Async::Ready(mut srv) = new_srv.new_service().poll().unwrap() {
            let res = srv.call("srv1").poll();
            assert!(res.is_ok());
            assert_eq!(res.unwrap(), Async::Ready(("srv1", "srv2")));
        } else {
            panic!()
        }
    }
}
