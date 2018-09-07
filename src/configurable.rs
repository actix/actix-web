use std::marker;

use futures::{future, future::FutureResult, Async, Future, IntoFuture, Poll};
use tower_service::Service;

use service::{AndThen, FnService, MapErr};

/// Creates new `Service` values.
///
/// Acts as a service factory. This is useful for cases where new `Service`
/// values must be produced. One case is a TCP servier listener. The listner
/// accepts new TCP streams, obtains a new `Service` value using the
/// `NewConfigurableService` trait, and uses that new `Service` value to
/// process inbound requests on that new TCP stream.
pub trait NewConfigurableService<C> {
    /// Requests handled by the service
    type Request;

    /// Responses given by the service
    type Response;

    /// Errors produced by the service
    type Error;

    /// The `Service` value created by this factory
    type Service: Service<
        Request = Self::Request,
        Response = Self::Response,
        Error = Self::Error,
    >;

    /// Errors produced while building a service.
    type InitError;

    /// The future of the `Service` instance.
    type Future: Future<Item = Self::Service, Error = Self::InitError>;

    /// Create and return a new service value asynchronously.
    fn new_service(&self, C) -> Self::Future;

    fn and_then<F, B>(self, new_service: F) -> AndThenNewConfigurableService<Self, B, C>
    where
        Self: Sized,
        F: IntoNewConfigurableService<B, C>,
        B: NewConfigurableService<
            C,
            Request = Self::Response,
            Error = Self::Error,
            InitError = Self::InitError,
        >,
    {
        AndThenNewConfigurableService::new(self, new_service)
    }

    fn map_err<F, E>(self, f: F) -> MapErrNewConfigurableService<Self, F, E, C>
    where
        Self: Sized,
        F: Fn(Self::Error) -> E,
    {
        MapErrNewConfigurableService::new(self, f)
    }

    fn map_init_err<F, E>(self, f: F) -> MapInitErr<Self, F, E, C>
    where
        Self: Sized,
        F: Fn(Self::InitError) -> E,
    {
        MapInitErr::new(self, f)
    }
}

/// Trait for types that can be converted to a Service
pub trait IntoNewConfigurableService<T, C>
where
    T: NewConfigurableService<C>,
{
    /// Create service
    fn into_new_service(self) -> T;
}

impl<T, C> IntoNewConfigurableService<T, C> for T
where
    T: NewConfigurableService<C>,
{
    fn into_new_service(self) -> T {
        self
    }
}

pub struct Fn2NewConfigurableService<F, S, Err, Fut, Cfg>
where
    S: Service,
    F: Fn(Cfg) -> Fut,
    Fut: IntoFuture<Item = S, Error = Err>,
{
    f: F,
    err: marker::PhantomData<Err>,
    cfg: marker::PhantomData<Cfg>,
    fut: marker::PhantomData<Fut>,
    s: marker::PhantomData<S>,
}

impl<F, S, Err, Fut, Cfg> Fn2NewConfigurableService<F, S, Err, Fut, Cfg>
where
    S: Service,
    F: Fn(Cfg) -> Fut + 'static,
    Fut: IntoFuture<Item = S, Error = Err>,
{
    fn new(f: F) -> Self {
        Fn2NewConfigurableService {
            f,
            err: marker::PhantomData,
            cfg: marker::PhantomData,
            fut: marker::PhantomData,
            s: marker::PhantomData,
        }
    }
}

impl<F, S, Err, Fut, Cfg>
    IntoNewConfigurableService<Fn2NewConfigurableService<F, S, Err, Fut, Cfg>, Cfg> for F
where
    S: Service,
    F: Fn(Cfg) -> Fut + 'static,
    Fut: IntoFuture<Item = S, Error = Err>,
{
    fn into_new_service(self) -> Fn2NewConfigurableService<F, S, Err, Fut, Cfg> {
        Fn2NewConfigurableService::new(self)
    }
}

impl<F, S, Err, Fut, Cfg> Clone for Fn2NewConfigurableService<F, S, Err, Fut, Cfg>
where
    S: Service,
    F: Fn(Cfg) -> Fut + Clone + 'static,
    Fut: IntoFuture<Item = S, Error = Err>,
{
    fn clone(&self) -> Self {
        Self::new(self.f.clone())
    }
}

impl<F, S, Err, Fut, Cfg> NewConfigurableService<Cfg>
    for Fn2NewConfigurableService<F, S, Err, Fut, Cfg>
where
    S: Service,
    F: Fn(Cfg) -> Fut,
    Fut: IntoFuture<Item = S, Error = Err>,
{
    type Request = S::Request;
    type Response = S::Response;
    type Error = S::Error;
    type Service = S;
    type InitError = Err;
    type Future = Fut::Future;

    fn new_service(&self, cfg: Cfg) -> Self::Future {
        (self.f)(cfg).into_future()
    }
}

pub struct FnNewConfigurableService<F, Req, Resp, Err, IErr, Fut, Cfg>
where
    F: Fn(Req) -> Fut,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    f: F,
    req: marker::PhantomData<Req>,
    resp: marker::PhantomData<Resp>,
    err: marker::PhantomData<Err>,
    ierr: marker::PhantomData<IErr>,
    cfg: marker::PhantomData<Cfg>,
}

impl<F, Req, Resp, Err, IErr, Fut, Cfg>
    FnNewConfigurableService<F, Req, Resp, Err, IErr, Fut, Cfg>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    fn new(f: F) -> Self {
        FnNewConfigurableService {
            f,
            req: marker::PhantomData,
            resp: marker::PhantomData,
            err: marker::PhantomData,
            ierr: marker::PhantomData,
            cfg: marker::PhantomData,
        }
    }
}

impl<F, Req, Resp, Err, IErr, Fut, Cfg> NewConfigurableService<Cfg>
    for FnNewConfigurableService<F, Req, Resp, Err, IErr, Fut, Cfg>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    type Request = Req;
    type Response = Resp;
    type Error = Err;
    type Service = FnService<F, Req, Resp, Err, Fut>;
    type InitError = IErr;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self, _: Cfg) -> Self::Future {
        future::ok(FnService::new(self.f.clone()))
    }
}

impl<F, Req, Resp, Err, IErr, Fut, Cfg>
    IntoNewConfigurableService<FnNewConfigurableService<F, Req, Resp, Err, IErr, Fut, Cfg>, Cfg>
    for F
where
    F: Fn(Req) -> Fut + Clone + 'static,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    fn into_new_service(self) -> FnNewConfigurableService<F, Req, Resp, Err, IErr, Fut, Cfg> {
        FnNewConfigurableService::new(self)
    }
}

impl<F, Req, Resp, Err, IErr, Fut, Cfg> Clone
    for FnNewConfigurableService<F, Req, Resp, Err, IErr, Fut, Cfg>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    fn clone(&self) -> Self {
        Self::new(self.f.clone())
    }
}

/// `AndThenNewConfigurableService` new service combinator
pub struct AndThenNewConfigurableService<A, B, C>
where
    A: NewConfigurableService<C>,
    B: NewConfigurableService<C>,
{
    a: A,
    b: B,
    c: marker::PhantomData<C>,
}

impl<A, B, C> AndThenNewConfigurableService<A, B, C>
where
    A: NewConfigurableService<C>,
    B: NewConfigurableService<C>,
{
    /// Create new `AndThen` combinator
    pub fn new<F: IntoNewConfigurableService<B, C>>(a: A, f: F) -> Self {
        Self {
            a,
            b: f.into_new_service(),
            c: marker::PhantomData,
        }
    }
}

impl<A, B, C> NewConfigurableService<C> for AndThenNewConfigurableService<A, B, C>
where
    A: NewConfigurableService<C>,
    B: NewConfigurableService<
        C,
        Request = A::Response,
        Error = A::Error,
        InitError = A::InitError,
    >,
    C: Clone,
{
    type Request = A::Request;
    type Response = B::Response;
    type Error = A::Error;
    type Service = AndThen<A::Service, B::Service>;

    type InitError = A::InitError;
    type Future = AndThenNewConfigurableServiceFuture<A, B, C>;

    fn new_service(&self, cfg: C) -> Self::Future {
        AndThenNewConfigurableServiceFuture::new(
            self.a.new_service(cfg.clone()),
            self.b.new_service(cfg),
        )
    }
}

impl<A, B, C> Clone for AndThenNewConfigurableService<A, B, C>
where
    A: NewConfigurableService<C> + Clone,
    B: NewConfigurableService<
            C,
            Request = A::Response,
            Error = A::Error,
            InitError = A::InitError,
        > + Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            b: self.b.clone(),
            c: marker::PhantomData,
        }
    }
}

pub struct AndThenNewConfigurableServiceFuture<A, B, C>
where
    A: NewConfigurableService<C>,
    B: NewConfigurableService<C>,
{
    fut_b: B::Future,
    fut_a: A::Future,
    a: Option<A::Service>,
    b: Option<B::Service>,
    c: marker::PhantomData<C>,
}

impl<A, B, C> AndThenNewConfigurableServiceFuture<A, B, C>
where
    A: NewConfigurableService<C>,
    B: NewConfigurableService<C>,
{
    fn new(fut_a: A::Future, fut_b: B::Future) -> Self {
        AndThenNewConfigurableServiceFuture {
            fut_a,
            fut_b,
            a: None,
            b: None,
            c: marker::PhantomData,
        }
    }
}

impl<A, B, C> Future for AndThenNewConfigurableServiceFuture<A, B, C>
where
    A: NewConfigurableService<C>,
    B: NewConfigurableService<
        C,
        Request = A::Response,
        Error = A::Error,
        InitError = A::InitError,
    >,
{
    type Item = AndThen<A::Service, B::Service>;
    type Error = A::InitError;

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

/// `MapErrNewService` new service combinator
pub struct MapErrNewConfigurableService<A, F, E, C>
where
    A: NewConfigurableService<C>,
{
    a: A,
    f: F,
    e: marker::PhantomData<E>,
    c: marker::PhantomData<C>,
}

impl<A, F, E, C> MapErrNewConfigurableService<A, F, E, C>
where
    A: NewConfigurableService<C>,
    F: Fn(A::Error) -> E,
{
    /// Create new `MapErr` new service instance
    pub fn new(a: A, f: F) -> Self {
        Self {
            a,
            f,
            e: marker::PhantomData,
            c: marker::PhantomData,
        }
    }
}

impl<A, F, E, C> Clone for MapErrNewConfigurableService<A, F, E, C>
where
    A: NewConfigurableService<C> + Clone,
    F: Fn(A::Error) -> E + Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            f: self.f.clone(),
            e: marker::PhantomData,
            c: marker::PhantomData,
        }
    }
}

impl<A, F, E, C> NewConfigurableService<C> for MapErrNewConfigurableService<A, F, E, C>
where
    A: NewConfigurableService<C> + Clone,
    F: Fn(A::Error) -> E + Clone,
{
    type Request = A::Request;
    type Response = A::Response;
    type Error = E;
    type Service = MapErr<A::Service, F, E>;

    type InitError = A::InitError;
    type Future = MapErrNewConfigurableServiceFuture<A, F, E, C>;

    fn new_service(&self, cfg: C) -> Self::Future {
        MapErrNewConfigurableServiceFuture::new(self.a.new_service(cfg), self.f.clone())
    }
}

pub struct MapErrNewConfigurableServiceFuture<A, F, E, C>
where
    A: NewConfigurableService<C>,
    F: Fn(A::Error) -> E,
{
    fut: A::Future,
    f: F,
}

impl<A, F, E, C> MapErrNewConfigurableServiceFuture<A, F, E, C>
where
    A: NewConfigurableService<C>,
    F: Fn(A::Error) -> E,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapErrNewConfigurableServiceFuture { f, fut }
    }
}

impl<A, F, E, C> Future for MapErrNewConfigurableServiceFuture<A, F, E, C>
where
    A: NewConfigurableService<C>,
    F: Fn(A::Error) -> E + Clone,
{
    type Item = MapErr<A::Service, F, E>;
    type Error = A::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Async::Ready(service) = self.fut.poll()? {
            Ok(Async::Ready(MapErr::new(service, self.f.clone())))
        } else {
            Ok(Async::NotReady)
        }
    }
}

/// `MapInitErr` service combinator
pub struct MapInitErr<A, F, E, C>
where
    A: NewConfigurableService<C>,
{
    a: A,
    f: F,
    e: marker::PhantomData<E>,
    c: marker::PhantomData<C>,
}

impl<A, F, E, C> MapInitErr<A, F, E, C>
where
    A: NewConfigurableService<C>,
    F: Fn(A::InitError) -> E,
{
    /// Create new `MapInitErr` combinator
    pub fn new(a: A, f: F) -> Self {
        Self {
            a,
            f,
            e: marker::PhantomData,
            c: marker::PhantomData,
        }
    }
}

impl<A, F, E, C> Clone for MapInitErr<A, F, E, C>
where
    A: NewConfigurableService<C> + Clone,
    F: Fn(A::InitError) -> E + Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            f: self.f.clone(),
            e: marker::PhantomData,
            c: marker::PhantomData,
        }
    }
}

impl<A, F, E, C> NewConfigurableService<C> for MapInitErr<A, F, E, C>
where
    A: NewConfigurableService<C>,
    F: Fn(A::InitError) -> E + Clone,
{
    type Request = A::Request;
    type Response = A::Response;
    type Error = A::Error;
    type Service = A::Service;

    type InitError = E;
    type Future = MapInitErrFuture<A, F, E, C>;

    fn new_service(&self, cfg: C) -> Self::Future {
        MapInitErrFuture::new(self.a.new_service(cfg), self.f.clone())
    }
}

pub struct MapInitErrFuture<A, F, E, C>
where
    A: NewConfigurableService<C>,
    F: Fn(A::InitError) -> E,
{
    f: F,
    fut: A::Future,
}

impl<A, F, E, C> MapInitErrFuture<A, F, E, C>
where
    A: NewConfigurableService<C>,
    F: Fn(A::InitError) -> E,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapInitErrFuture { f, fut }
    }
}

impl<A, F, E, C> Future for MapInitErrFuture<A, F, E, C>
where
    A: NewConfigurableService<C>,
    F: Fn(A::InitError) -> E,
{
    type Item = A::Service;
    type Error = E;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.fut.poll().map_err(&self.f)
    }
}
