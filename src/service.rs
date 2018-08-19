use std::cell::RefCell;
use std::marker;
use std::rc::Rc;

use futures::{future, future::FutureResult, Async, Future, IntoFuture, Poll};
use tower_service::{NewService, Service};

pub trait NewServiceExt: NewService {
    fn and_then<F, B>(self, new_service: F) -> AndThenNewService<Self, B>
    where
        Self: Sized,
        F: IntoNewService<B>,
        B: NewService<
            Request = Self::Response,
            Error = Self::Error,
            InitError = Self::InitError,
        >;
}

impl<T> NewServiceExt for T
where
    T: NewService,
{
    fn and_then<F, B>(self, new_service: F) -> AndThenNewService<Self, B>
    where
        F: IntoNewService<B>,
        B: NewService<
            Request = Self::Response,
            Error = Self::Error,
            InitError = Self::InitError,
        >,
    {
        AndThenNewService::new(self, new_service)
    }
}

/// Trait for types that can be converted to a Service
pub trait IntoService<T>
where
    T: Service,
{
    /// Create service
    fn into(self) -> T;
}

/// Trait for types that can be converted to a Service
pub trait IntoNewService<T>
where
    T: NewService,
{
    /// Create service
    fn into(self) -> T;
}

impl<T> IntoService<T> for T
where
    T: Service,
{
    fn into(self) -> T {
        self
    }
}

impl<T> IntoNewService<T> for T
where
    T: NewService,
{
    fn into(self) -> T {
        self
    }
}

impl<F, Req, Resp, Err, Fut> IntoService<FnService<F, Req, Resp, Err, Fut>> for F
where
    F: Fn(Req) -> Fut + 'static,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    fn into(self) -> FnService<F, Req, Resp, Err, Fut> {
        FnService::new(self)
    }
}

pub struct FnService<F, Req, Resp, E, Fut>
where
    F: Fn(Req) -> Fut,
    Fut: IntoFuture<Item = Resp, Error = E>,
{
    f: F,
    req: marker::PhantomData<Req>,
    resp: marker::PhantomData<Resp>,
    err: marker::PhantomData<E>,
}

impl<F, Req, Resp, E, Fut> FnService<F, Req, Resp, E, Fut>
where
    F: Fn(Req) -> Fut,
    Fut: IntoFuture<Item = Resp, Error = E>,
{
    pub fn new(f: F) -> Self {
        FnService {
            f,
            req: marker::PhantomData,
            resp: marker::PhantomData,
            err: marker::PhantomData,
        }
    }
}

impl<F, Req, Resp, E, Fut> Service for FnService<F, Req, Resp, E, Fut>
where
    F: Fn(Req) -> Fut,
    Fut: IntoFuture<Item = Resp, Error = E>,
{
    type Request = Req;
    type Response = Resp;
    type Error = E;
    type Future = Fut::Future;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Req) -> Self::Future {
        (self.f)(req).into_future()
    }
}

pub struct FnNewService<F, Req, Resp, Err, Fut>
where
    F: Fn(Req) -> Fut,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    f: F,
    req: marker::PhantomData<Req>,
    resp: marker::PhantomData<Resp>,
    err: marker::PhantomData<Err>,
}

impl<F, Req, Resp, Err, Fut> FnNewService<F, Req, Resp, Err, Fut>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    fn new(f: F) -> Self {
        FnNewService {
            f,
            req: marker::PhantomData,
            resp: marker::PhantomData,
            err: marker::PhantomData,
        }
    }
}

impl<F, Req, Resp, Err, Fut> NewService for FnNewService<F, Req, Resp, Err, Fut>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    type Request = Req;
    type Response = Resp;
    type Error = Err;
    type Service = FnService<F, Req, Resp, Err, Fut>;
    type InitError = ();
    type Future = FutureResult<Self::Service, ()>;

    fn new_service(&self) -> Self::Future {
        future::ok(FnService::new(self.f.clone()))
    }
}

impl<F, Req, Resp, Err, Fut> IntoNewService<FnNewService<F, Req, Resp, Err, Fut>> for F
where
    F: Fn(Req) -> Fut + Clone + 'static,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    fn into(self) -> FnNewService<F, Req, Resp, Err, Fut> {
        FnNewService::new(self)
    }
}

impl<F, Req, Resp, Err, Fut> Clone for FnNewService<F, Req, Resp, Err, Fut>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    fn clone(&self) -> Self {
        Self::new(self.f.clone())
    }
}

pub struct FnStateService<S, F, Req, Resp, Err, Fut>
where
    F: Fn(&mut S, Req) -> Fut,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    f: F,
    state: S,
    req: marker::PhantomData<Req>,
    resp: marker::PhantomData<Resp>,
    err: marker::PhantomData<Err>,
}

impl<S, F, Req, Resp, Err, Fut> FnStateService<S, F, Req, Resp, Err, Fut>
where
    F: Fn(&mut S, Req) -> Fut,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    pub fn new(state: S, f: F) -> Self {
        FnStateService {
            f,
            state,
            req: marker::PhantomData,
            resp: marker::PhantomData,
            err: marker::PhantomData,
        }
    }
}

impl<S, F, Req, Resp, Err, Fut> Service for FnStateService<S, F, Req, Resp, Err, Fut>
where
    F: Fn(&mut S, Req) -> Fut,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    type Request = Req;
    type Response = Resp;
    type Error = Err;
    type Future = Fut::Future;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Req) -> Self::Future {
        (self.f)(&mut self.state, req).into_future()
    }
}

/// `NewService` for state and handler functions
pub struct FnStateNewService<S, F1, F2, Req, Resp, Err1, Err2, Fut1, Fut2> {
    f: F1,
    state: F2,
    s: marker::PhantomData<S>,
    req: marker::PhantomData<Req>,
    resp: marker::PhantomData<Resp>,
    err1: marker::PhantomData<Err1>,
    err2: marker::PhantomData<Err2>,
    fut1: marker::PhantomData<Fut1>,
    fut2: marker::PhantomData<Fut2>,
}

impl<S, F1, F2, Req, Resp, Err1, Err2, Fut1, Fut2>
    FnStateNewService<S, F1, F2, Req, Resp, Err1, Err2, Fut1, Fut2>
{
    fn new(f: F1, state: F2) -> Self {
        FnStateNewService {
            f,
            state,
            s: marker::PhantomData,
            req: marker::PhantomData,
            resp: marker::PhantomData,
            err1: marker::PhantomData,
            err2: marker::PhantomData,
            fut1: marker::PhantomData,
            fut2: marker::PhantomData,
        }
    }
}

impl<S, F1, F2, Req, Resp, Err1, Err2, Fut1, Fut2> NewService
    for FnStateNewService<S, F1, F2, Req, Resp, Err1, Err2, Fut1, Fut2>
where
    S: 'static,
    F1: Fn(&mut S, Req) -> Fut1 + Clone + 'static,
    F2: Fn() -> Fut2,
    Fut1: IntoFuture<Item = Resp, Error = Err1> + 'static,
    Fut2: IntoFuture<Item = S, Error = Err2> + 'static,
    Req: 'static,
    Resp: 'static,
    Err1: 'static,
    Err2: 'static,
{
    type Request = Req;
    type Response = Resp;
    type Error = Err1;
    type Service = FnStateService<S, F1, Req, Resp, Err1, Fut1>;
    type InitError = Err2;
    type Future = Box<Future<Item = Self::Service, Error = Self::InitError>>;

    fn new_service(&self) -> Self::Future {
        let f = self.f.clone();
        Box::new(
            (self.state)()
                .into_future()
                .and_then(move |state| Ok(FnStateService::new(state, f))),
        )
    }
}

impl<S, F1, F2, Req, Resp, Err1, Err2, Fut1, Fut2>
    IntoNewService<FnStateNewService<S, F1, F2, Req, Resp, Err1, Err2, Fut1, Fut2>> for (F1, F2)
where
    S: 'static,
    F1: Fn(&mut S, Req) -> Fut1 + Clone + 'static,
    F2: Fn() -> Fut2,
    Fut1: IntoFuture<Item = Resp, Error = Err1> + 'static,
    Fut2: IntoFuture<Item = S, Error = Err2> + 'static,
    Req: 'static,
    Resp: 'static,
    Err1: 'static,
    Err2: 'static,
{
    fn into(self) -> FnStateNewService<S, F1, F2, Req, Resp, Err1, Err2, Fut1, Fut2> {
        FnStateNewService::new(self.0, self.1)
    }
}

impl<S, F1, F2, Req, Resp, Err1, Err2, Fut1, Fut2> Clone
    for FnStateNewService<S, F1, F2, Req, Resp, Err1, Err2, Fut1, Fut2>
where
    F1: Fn(&mut S, Req) -> Fut1 + Clone + 'static,
    F2: Fn() -> Fut2 + Clone,
    Fut1: IntoFuture<Item = Resp, Error = Err1>,
    Fut2: IntoFuture<Item = S, Error = Err2>,
{
    fn clone(&self) -> Self {
        Self::new(self.f.clone(), self.state.clone())
    }
}

/// `AndThen` service combinator
pub struct AndThen<A, B> {
    a: A,
    b: Rc<RefCell<B>>,
}

impl<A, B> AndThen<A, B>
where
    A: Service,
    B: Service<Request = A::Response, Error = A::Error>,
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
    B: Service<Request = A::Response, Error = A::Error>,
{
    type Request = A::Request;
    type Response = B::Response;
    type Error = B::Error;
    type Future = AndThenFuture<A, B>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        let res = self.a.poll_ready();
        if let Ok(Async::Ready(_)) = res {
            self.b.borrow_mut().poll_ready()
        } else {
            res
        }
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        AndThenFuture::new(self.a.call(req), self.b.clone())
    }
}

pub struct AndThenFuture<A, B>
where
    A: Service,
    B: Service<Request = A::Response, Error = A::Error>,
{
    b: Rc<RefCell<B>>,
    fut_b: Option<B::Future>,
    fut_a: A::Future,
}

impl<A, B> AndThenFuture<A, B>
where
    A: Service,
    B: Service<Request = A::Response, Error = A::Error>,
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
    B: Service<Request = A::Response, Error = A::Error>,
{
    type Item = B::Response;
    type Error = B::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut_b {
            return fut.poll();
        }

        match self.fut_a.poll()? {
            Async::Ready(resp) => {
                self.fut_b = Some(self.b.borrow_mut().call(resp));
                self.poll()
            }
            Async::NotReady => Ok(Async::NotReady),
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
    pub fn new<F: IntoNewService<B>>(a: A, f: F) -> Self {
        Self { a, b: f.into() }
    }
}

impl<A, B> NewService for AndThenNewService<A, B>
where
    A: NewService<Response = B::Request, Error = B::Error, InitError = B::InitError>,
    B: NewService,
{
    type Request = A::Request;
    type Response = B::Response;
    type Error = A::Error;
    type Service = AndThen<A::Service, B::Service>;

    type InitError = A::InitError;
    type Future = AndThenNewServiceFuture<A, B>;

    fn new_service(&self) -> Self::Future {
        AndThenNewServiceFuture::new(self.a.new_service(), self.b.new_service())
    }
}

impl<A, B> Clone for AndThenNewService<A, B>
where
    A: NewService<Response = B::Request, Error = B::Error, InitError = B::InitError> + Clone,
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
    A: NewService<Response = B::Request, Error = B::Error, InitError = B::InitError>,
    B: NewService,
{
    type Item = AndThen<A::Service, B::Service>;
    type Error = B::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Async::Ready(service) = self.fut_a.poll()? {
            self.a = Some(service);
        }

        if let Async::Ready(service) = self.fut_b.poll()? {
            self.b = Some(service);
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
