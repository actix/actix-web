use futures::{try_ready, Async, Future, Poll};

use super::{IntoNewService, NewService, Service};
use crate::cell::Cell;

/// Service for the `then` combinator, chaining a computation onto the end of
/// another service.
///
/// This is created by the `ServiceExt::then` method.
pub struct Then<A, B> {
    a: A,
    b: Cell<B>,
}

impl<A, B> Then<A, B> {
    /// Create new `Then` combinator
    pub fn new<Request>(a: A, b: B) -> Then<A, B>
    where
        A: Service<Request>,
        B: Service<Result<A::Response, A::Error>, Error = A::Error>,
    {
        Then { a, b: Cell::new(b) }
    }
}

impl<A, B> Clone for Then<A, B>
where
    A: Clone,
{
    fn clone(&self) -> Self {
        Then {
            a: self.a.clone(),
            b: self.b.clone(),
        }
    }
}

impl<A, B, Request> Service<Request> for Then<A, B>
where
    A: Service<Request>,
    B: Service<Result<A::Response, A::Error>, Error = A::Error>,
{
    type Response = B::Response;
    type Error = B::Error;
    type Future = ThenFuture<A, B, Request>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        try_ready!(self.a.poll_ready());
        self.b.get_mut().poll_ready()
    }

    fn call(&mut self, req: Request) -> Self::Future {
        ThenFuture::new(self.a.call(req), self.b.clone())
    }
}

pub struct ThenFuture<A, B, Request>
where
    A: Service<Request>,
    B: Service<Result<A::Response, A::Error>>,
{
    b: Cell<B>,
    fut_b: Option<B::Future>,
    fut_a: A::Future,
}

impl<A, B, Request> ThenFuture<A, B, Request>
where
    A: Service<Request>,
    B: Service<Result<A::Response, A::Error>>,
{
    fn new(fut_a: A::Future, b: Cell<B>) -> Self {
        ThenFuture {
            b,
            fut_a,
            fut_b: None,
        }
    }
}

impl<A, B, Request> Future for ThenFuture<A, B, Request>
where
    A: Service<Request>,
    B: Service<Result<A::Response, A::Error>>,
{
    type Item = B::Response;
    type Error = B::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut_b {
            return fut.poll();
        }

        match self.fut_a.poll() {
            Ok(Async::Ready(resp)) => {
                self.fut_b = Some(self.b.get_mut().call(Ok(resp)));
                self.poll()
            }
            Err(err) => {
                self.fut_b = Some(self.b.get_mut().call(Err(err)));
                self.poll()
            }
            Ok(Async::NotReady) => Ok(Async::NotReady),
        }
    }
}

/// `ThenNewService` new service combinator
pub struct ThenNewService<A, B> {
    a: A,
    b: B,
}

impl<A, B> ThenNewService<A, B> {
    /// Create new `AndThen` combinator
    pub fn new<F, Request>(a: A, f: F) -> Self
    where
        A: NewService<Request>,
        B: NewService<
            Result<A::Response, A::Error>,
            Error = A::Error,
            InitError = A::InitError,
        >,
        F: IntoNewService<B, Result<A::Response, A::Error>>,
    {
        Self {
            a,
            b: f.into_new_service(),
        }
    }
}

impl<A, B, Request> NewService<Request> for ThenNewService<A, B>
where
    A: NewService<Request>,
    B: NewService<Result<A::Response, A::Error>, Error = A::Error, InitError = A::InitError>,
{
    type Response = B::Response;
    type Error = A::Error;
    type Service = Then<A::Service, B::Service>;

    type InitError = A::InitError;
    type Future = ThenNewServiceFuture<A, B, Request>;

    fn new_service(&self) -> Self::Future {
        ThenNewServiceFuture::new(self.a.new_service(), self.b.new_service())
    }
}

impl<A, B> Clone for ThenNewService<A, B>
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

pub struct ThenNewServiceFuture<A, B, Request>
where
    A: NewService<Request>,
    B: NewService<Result<A::Response, A::Error>, Error = A::Error, InitError = A::InitError>,
{
    fut_b: B::Future,
    fut_a: A::Future,
    a: Option<A::Service>,
    b: Option<B::Service>,
}

impl<A, B, Request> ThenNewServiceFuture<A, B, Request>
where
    A: NewService<Request>,
    B: NewService<Result<A::Response, A::Error>, Error = A::Error, InitError = A::InitError>,
{
    fn new(fut_a: A::Future, fut_b: B::Future) -> Self {
        ThenNewServiceFuture {
            fut_a,
            fut_b,
            a: None,
            b: None,
        }
    }
}

impl<A, B, Request> Future for ThenNewServiceFuture<A, B, Request>
where
    A: NewService<Request>,
    B: NewService<Result<A::Response, A::Error>, Error = A::Error, InitError = A::InitError>,
{
    type Item = Then<A::Service, B::Service>;
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
            Ok(Async::Ready(Then::new(
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
    use futures::future::{err, ok, FutureResult};
    use futures::{Async, Poll};
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;

    #[derive(Clone)]
    struct Srv1(Rc<Cell<usize>>);
    impl Service<Result<&'static str, &'static str>> for Srv1 {
        type Response = &'static str;
        type Error = ();
        type Future = FutureResult<Self::Response, Self::Error>;

        fn poll_ready(&mut self) -> Poll<(), Self::Error> {
            self.0.set(self.0.get() + 1);
            Ok(Async::Ready(()))
        }

        fn call(&mut self, req: Result<&'static str, &'static str>) -> Self::Future {
            match req {
                Ok(msg) => ok(msg),
                Err(_) => err(()),
            }
        }
    }

    struct Srv2(Rc<Cell<usize>>);

    impl Service<Result<&'static str, ()>> for Srv2 {
        type Response = (&'static str, &'static str);
        type Error = ();
        type Future = FutureResult<Self::Response, ()>;

        fn poll_ready(&mut self) -> Poll<(), Self::Error> {
            self.0.set(self.0.get() + 1);
            Ok(Async::Ready(()))
        }

        fn call(&mut self, req: Result<&'static str, ()>) -> Self::Future {
            match req {
                Ok(msg) => ok((msg, "ok")),
                Err(()) => ok(("srv2", "err")),
            }
        }
    }

    #[test]
    fn test_poll_ready() {
        let cnt = Rc::new(Cell::new(0));
        let mut srv = Srv1(cnt.clone()).then(Srv2(cnt.clone()));
        let res = srv.poll_ready();
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), Async::Ready(()));
        assert_eq!(cnt.get(), 2);
    }

    #[test]
    fn test_call() {
        let cnt = Rc::new(Cell::new(0));
        let mut srv = Srv1(cnt.clone()).then(Srv2(cnt)).clone();

        let res = srv.call(Ok("srv1")).poll();
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), Async::Ready(("srv1", "ok")));

        let res = srv.call(Err("srv")).poll();
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), Async::Ready(("srv2", "err")));
    }

    #[test]
    fn test_new_service() {
        let cnt = Rc::new(Cell::new(0));
        let cnt2 = cnt.clone();
        let blank = move || Ok::<_, ()>(Srv1(cnt2.clone()));
        let new_srv = blank.into_new_service().then(move || Ok(Srv2(cnt.clone())));
        if let Async::Ready(mut srv) = new_srv.clone().new_service().poll().unwrap() {
            let res = srv.call(Ok("srv1")).poll();
            assert!(res.is_ok());
            assert_eq!(res.unwrap(), Async::Ready(("srv1", "ok")));

            let res = srv.call(Err("srv")).poll();
            assert!(res.is_ok());
            assert_eq!(res.unwrap(), Async::Ready(("srv2", "err")));
        } else {
            panic!()
        }
    }
}
