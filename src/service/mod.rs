use futures::{Future, IntoFuture};

/// re-export for convinience
pub use tower_service::Service;

mod and_then;
mod apply;
mod fn_service;
mod from_err;
mod map;
mod map_err;
mod map_init_err;
mod then;

pub use self::and_then::{AndThen, AndThenNewService};
pub use self::apply::{Apply, ApplyNewService};
pub use self::fn_service::{FnNewService, FnService};
pub use self::from_err::{FromErr, FromErrNewService};
pub use self::map::{Map, MapNewService};
pub use self::map_err::{MapErr, MapErrNewService};
pub use self::map_init_err::MapInitErr;
pub use self::then::{Then, ThenNewService};

/// An extension trait for `Service`s that provides a variety of convenient
/// adapters
pub trait ServiceExt<Request>: Service<Request> {
    /// Apply function to specified service and use it as a next service in
    /// chain.
    fn apply<T, I, F, Out, Req>(
        self,
        service: I,
        f: F,
    ) -> AndThen<Self, Apply<T, F, Self::Response, Out, Req>>
    where
        Self: Sized,
        T: Service<Req, Error = Out::Error>,
        I: IntoService<T, Req>,
        F: Fn(Self::Response, &mut T) -> Out,
        Out: IntoFuture<Error = Self::Error>,
    {
        self.and_then(Apply::new(service.into_service(), f))
    }

    /// Call another service after call to this one has resolved successfully.
    ///
    /// This function can be used to chain two services together and ensure that
    /// the second service isn't called until call to the fist service have
    /// finished. Result of the call to the first service is used as an
    /// input parameter for the second service's call.
    ///
    /// Note that this function consumes the receiving service and returns a
    /// wrapped version of it.
    fn and_then<F, B>(self, service: F) -> AndThen<Self, B>
    where
        Self: Sized,
        F: IntoService<B, Self::Response>,
        B: Service<Self::Response, Error = Self::Error>,
    {
        AndThen::new(self, service.into_service())
    }

    // /// Map this service's error to any error implementing `From` for
    // /// this service`s `Error`.
    // ///
    // /// Note that this function consumes the receiving service and returns a
    // /// wrapped version of it.
    fn from_err<E>(self) -> FromErr<Self, E>
    where
        Self: Sized,
        E: From<Self::Error>,
    {
        FromErr::new(self)
    }

    // /// Chain on a computation for when a call to the service finished,
    // /// passing the result of the call to the next service `B`.
    // ///
    // /// Note that this function consumes the receiving future and returns a
    // /// wrapped version of it.
    fn then<B>(self, service: B) -> Then<Self, B>
    where
        Self: Sized,
        B: Service<Result<Self::Response, Self::Error>, Error = Self::Error>,
    {
        Then::new(self, service)
    }

    // /// Map this service's output to a different type, returning a new service
    // /// of the resulting type.
    // ///
    // /// This function is similar to the `Option::map` or `Iterator::map` where
    // /// it will change the type of the underlying service.
    // ///
    // /// Note that this function consumes the receiving service and returns a
    // /// wrapped version of it, similar to the existing `map` methods in the
    // /// standard library.
    fn map<F, R>(self, f: F) -> Map<Self, F, R>
    where
        Self: Sized,
        F: Fn(Self::Response) -> R,
    {
        Map::new(self, f)
    }

    // /// Map this service's error to a different error, returning a new service.
    // ///
    // /// This function is similar to the `Result::map_err` where it will change
    // /// the error type of the underlying service. This is useful for example to
    // /// ensure that services have the same error type.
    // ///
    // /// Note that this function consumes the receiving service and returns a
    // /// wrapped version of it.
    fn map_err<F, E>(self, f: F) -> MapErr<Self, F, E>
    where
        Self: Sized,
        F: Fn(Self::Error) -> E,
    {
        MapErr::new(self, f)
    }
}

/// Creates new `Service` values.
///
/// Acts as a service factory. This is useful for cases where new `Service`
/// values must be produced. One case is a TCP servier listener. The listner
/// accepts new TCP streams, obtains a new `Service` value using the
/// `NewService` trait, and uses that new `Service` value to process inbound
/// requests on that new TCP stream.
///
/// Request - request handled by the service
pub trait NewService<Request> {
    /// Responses given by the service
    type Response;

    /// Errors produced by the service
    type Error;

    /// The `Service` value created by this factory
    type Service: Service<Request, Response = Self::Response, Error = Self::Error>;

    /// Errors produced while building a service.
    type InitError;

    /// The future of the `Service` instance.
    type Future: Future<Item = Self::Service, Error = Self::InitError>;

    /// Create and return a new service value asynchronously.
    fn new_service(&self) -> Self::Future;
}

pub trait NewServiceExt<Request>: NewService<Request> {
    fn apply<T, I, F, Out, Req>(
        self,
        service: I,
        f: F,
    ) -> AndThenNewService<Self, ApplyNewService<T, F, Self::Response, Out, Req>>
    where
        Self: Sized,
        T: NewService<Req, InitError = Self::InitError, Error = Out::Error>,
        I: IntoNewService<T, Req>,
        F: Fn(Self::Response, &mut T::Service) -> Out + Clone,
        Out: IntoFuture<Error = Self::Error>,
    {
        self.and_then(ApplyNewService::new(service, f))
    }

    fn and_then<F, B>(self, new_service: F) -> AndThenNewService<Self, B>
    where
        Self: Sized,
        F: IntoNewService<B, Self::Response>,
        B: NewService<Self::Response, Error = Self::Error, InitError = Self::InitError>,
    {
        AndThenNewService::new(self, new_service)
    }

    // /// `NewService` that create service to map this service's error
    // /// and new service's init error to any error
    // /// implementing `From` for this service`s `Error`.
    // ///
    // /// Note that this function consumes the receiving new service and returns a
    // /// wrapped version of it.
    fn from_err<E>(self) -> FromErrNewService<Self, E>
    where
        Self: Sized,
        E: From<Self::Error>,
    {
        FromErrNewService::new(self)
    }

    // /// Create `NewService` to chain on a computation for when a call to the
    // /// service finished, passing the result of the call to the next
    // /// service `B`.
    // ///
    // /// Note that this function consumes the receiving future and returns a
    // /// wrapped version of it.
    fn then<F, B>(self, new_service: F) -> ThenNewService<Self, B>
    where
        Self: Sized,
        F: IntoNewService<B, Result<Self::Response, Self::Error>>,
        B: NewService<
            Result<Self::Response, Self::Error>,
            Error = Self::Error,
            InitError = Self::InitError,
        >,
    {
        ThenNewService::new(self, new_service)
    }

    fn map<F, R>(self, f: F) -> MapNewService<Self, F, R>
    where
        Self: Sized,
        F: Fn(Self::Response) -> R,
    {
        MapNewService::new(self, f)
    }

    fn map_err<F, E>(self, f: F) -> MapErrNewService<Self, F, E>
    where
        Self: Sized,
        F: Fn(Self::Error) -> E,
    {
        MapErrNewService::new(self, f)
    }

    fn map_init_err<F, E>(self, f: F) -> MapInitErr<Self, F, E>
    where
        Self: Sized,
        F: Fn(Self::InitError) -> E,
    {
        MapInitErr::new(self, f)
    }
}

impl<F, R, E, S, Request> NewService<Request> for F
where
    F: Fn() -> R,
    R: IntoFuture<Item = S, Error = E>,
    S: Service<Request>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Service = S;
    type InitError = E;
    type Future = R::Future;

    fn new_service(&self) -> Self::Future {
        (*self)().into_future()
    }
}

impl<T: ?Sized, R> ServiceExt<R> for T where T: Service<R> {}
impl<T: ?Sized, R> NewServiceExt<R> for T where T: NewService<R> {}

/// Trait for types that can be converted to a `Service`
pub trait IntoService<T, Request>
where
    T: Service<Request>,
{
    /// Convert to a `Service`
    fn into_service(self) -> T;
}

/// Trait for types that can be converted to a Service
pub trait IntoNewService<T, Request>
where
    T: NewService<Request>,
{
    /// Convert to an `NewService`
    fn into_new_service(self) -> T;
}

impl<T, Request> IntoService<T, Request> for T
where
    T: Service<Request>,
{
    fn into_service(self) -> T {
        self
    }
}

impl<T, Request> IntoNewService<T, Request> for T
where
    T: NewService<Request>,
{
    fn into_new_service(self) -> T {
        self
    }
}
