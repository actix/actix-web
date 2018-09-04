use std::cell::RefCell;
use std::rc::Rc;

use futures::{Async, Future, Poll};
use tower_service::{NewService, Service};

/// `AndThen` service combinator
pub struct AndThen<A, B> {
    a: A,
    b: Rc<RefCell<B>>,
}

impl<A, B> AndThen<A, B>
where
    A: Service,
    A::Error: Into<B::Error>,
    B: Service<Request = A::Response>,
{
    /// Create new `AndThen` combinator
    pub fn new(a: A, b: B) -> Self {
        Self {
            a,
            b: Rc::new(RefCell::new(b)),
        }
    }
}

impl<A, B> Service for AndThen<A, B>
where
    A: Service,
    A::Error: Into<B::Error>,
    B: Service<Request = A::Response>,
{
    type Request = A::Request;
    type Response = B::Response;
    type Error = B::Error;
    type Future = AndThenFuture<A, B>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        match self.a.poll_ready() {
            Ok(Async::Ready(_)) => self.b.borrow_mut().poll_ready(),
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(err) => Err(err.into()),
        }
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        AndThenFuture::new(self.a.call(req), self.b.clone())
    }
}

pub struct AndThenFuture<A, B>
where
    A: Service,
    A::Error: Into<B::Error>,
    B: Service<Request = A::Response>,
{
    b: Rc<RefCell<B>>,
    fut_b: Option<B::Future>,
    fut_a: A::Future,
}

impl<A, B> AndThenFuture<A, B>
where
    A: Service,
    A::Error: Into<B::Error>,
    B: Service<Request = A::Response>,
{
    fn new(fut_a: A::Future, b: Rc<RefCell<B>>) -> Self {
        AndThenFuture {
            b,
            fut_a,
            fut_b: None,
        }
    }
}

impl<A, B> Future for AndThenFuture<A, B>
where
    A: Service,
    A::Error: Into<B::Error>,
    B: Service<Request = A::Response>,
{
    type Item = B::Response;
    type Error = B::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut_b {
            return fut.poll();
        }

        match self.fut_a.poll() {
            Ok(Async::Ready(resp)) => {
                self.fut_b = Some(self.b.borrow_mut().call(resp));
                self.poll()
            }
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(err) => Err(err.into()),
        }
    }
}

/// `AndThenNewService` new service combinator
pub struct AndThenNewService<A, B> {
    a: A,
    b: B,
}

impl<A, B> AndThenNewService<A, B>
where
    A: NewService,
    B: NewService,
{
    /// Create new `AndThen` combinator
    pub fn new<F: Into<B>>(a: A, f: F) -> Self {
        Self { a, b: f.into() }
    }
}

impl<A, B> NewService for AndThenNewService<A, B>
where
    A: NewService<Response = B::Request, InitError = B::InitError>,
    A::Error: Into<B::Error>,
    A::InitError: Into<B::InitError>,
    B: NewService,
{
    type Request = A::Request;
    type Response = B::Response;
    type Error = B::Error;
    type Service = AndThen<A::Service, B::Service>;

    type InitError = B::InitError;
    type Future = AndThenNewServiceFuture<A, B>;

    fn new_service(&self) -> Self::Future {
        AndThenNewServiceFuture::new(self.a.new_service(), self.b.new_service())
    }
}

impl<A, B> Clone for AndThenNewService<A, B>
where
    A: NewService<Response = B::Request, InitError = B::InitError> + Clone,
    A::Error: Into<B::Error>,
    A::InitError: Into<B::InitError>,
    B: NewService + Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            b: self.b.clone(),
        }
    }
}

pub struct AndThenNewServiceFuture<A, B>
where
    A: NewService,
    B: NewService,
{
    fut_b: B::Future,
    fut_a: A::Future,
    a: Option<A::Service>,
    b: Option<B::Service>,
}

impl<A, B> AndThenNewServiceFuture<A, B>
where
    A: NewService,
    B: NewService,
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

impl<A, B> Future for AndThenNewServiceFuture<A, B>
where
    A: NewService,
    A::Error: Into<B::Error>,
    A::InitError: Into<B::InitError>,
    B: NewService<Request = A::Response>,
{
    type Item = AndThen<A::Service, B::Service>;
    type Error = B::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if self.a.is_none() {
            if let Async::Ready(service) = self.fut_a.poll().map_err(|e| e.into())? {
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
