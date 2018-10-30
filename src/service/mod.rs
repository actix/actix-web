use futures::{Future, IntoFuture};

/// re-export for convinience
pub use tower_service::{NewService, Service};

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
pub trait ServiceExt: Service {
    /// Apply function to specified service and use it as a next service in
    /// chain.
    fn apply<S, I, F, R>(
        self,
        service: I,
        f: F,
    ) -> AndThen<Self, Apply<S, F, R, Self::Response>>
    where
        Self: Sized,
        S: Service,
        S::Error: Into<<R::Future as Future>::Error>,
        I: IntoService<S>,
        F: Fn(Self::Response, &mut S) -> R,
        R: IntoFuture<Error = Self::Error>,
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
        F: IntoService<B>,
        B: Service<Request = Self::Response, Error = Self::Error>,
    {
        AndThen::new(self, service.into_service())
    }

    /// Map this service's error to any error implementing `From` for
    /// this service`s `Error`.
    ///
    /// Note that this function consumes the receiving service and returns a
    /// wrapped version of it.
    fn from_err<E>(self) -> FromErr<Self, E>
    where
        Self: Sized,
        E: From<Self::Error>,
    {
        FromErr::new(self)
    }

    /// Chain on a computation for when a call to the service finished,
    /// passing the result of the call to the next service `B`.
    ///
    /// Note that this function consumes the receiving future and returns a
    /// wrapped version of it.
    fn then<B>(self, service: B) -> Then<Self, B>
    where
        Self: Sized,
        B: Service<Request = Result<Self::Response, Self::Error>, Error = Self::Error>,
    {
        Then::new(self, service)
    }

    /// Map this service's output to a different type, returning a new service
    /// of the resulting type.
    ///
    /// This function is similar to the `Option::map` or `Iterator::map` where
    /// it will change the type of the underlying service.
    ///
    /// Note that this function consumes the receiving service and returns a
    /// wrapped version of it, similar to the existing `map` methods in the
    /// standard library.
    fn map<F, R>(self, f: F) -> Map<Self, F, R>
    where
        Self: Sized,
        F: Fn(Self::Response) -> R,
    {
        Map::new(self, f)
    }

    /// Map this service's error to a different error, returning a new service.
    ///
    /// This function is similar to the `Result::map_err` where it will change
    /// the error type of the underlying service. This is useful for example to
    /// ensure that services have the same error type.
    ///
    /// Note that this function consumes the receiving service and returns a
    /// wrapped version of it.
    fn map_err<F, E>(self, f: F) -> MapErr<Self, F, E>
    where
        Self: Sized,
        F: Fn(Self::Error) -> E,
    {
        MapErr::new(self, f)
    }
}

pub trait NewServiceExt: NewService {
    fn apply<S, I, F, R>(
        self,
        service: I,
        f: F,
    ) -> AndThenNewService<Self, ApplyNewService<S, F, R, Self::Response>>
    where
        Self: Sized,
        S: NewService<InitError = Self::InitError>,
        S::Error: Into<<R::Future as Future>::Error>,
        I: IntoNewService<S>,
        F: Fn(Self::Response, &mut S::Service) -> R + Clone,
        R: IntoFuture<Error = Self::Error>,
    {
        self.and_then(ApplyNewService::new(service, f))
    }

    fn and_then<F, B>(self, new_service: F) -> AndThenNewService<Self, B>
    where
        Self: Sized,
        F: IntoNewService<B>,
        B: NewService<
            Request = Self::Response,
            Error = Self::Error,
            InitError = Self::InitError,
        >,
    {
        AndThenNewService::new(self, new_service)
    }

    /// `NewService` that create service to map this service's error
    /// and new service's init error to any error
    /// implementing `From` for this service`s `Error`.
    ///
    /// Note that this function consumes the receiving new service and returns a
    /// wrapped version of it.
    fn from_err<E>(self) -> FromErrNewService<Self, E>
    where
        Self: Sized,
        E: From<Self::Error>,
    {
        FromErrNewService::new(self)
    }

    /// Create `NewService` to chain on a computation for when a call to the
    /// service finished, passing the result of the call to the next
    /// service `B`.
    ///
    /// Note that this function consumes the receiving future and returns a
    /// wrapped version of it.
    fn then<F, B>(self, new_service: F) -> ThenNewService<Self, B>
    where
        Self: Sized,
        F: IntoNewService<B>,
        B: NewService<
            Request = Result<Self::Response, Self::Error>,
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

impl<T: ?Sized> ServiceExt for T where T: Service {}
impl<T: ?Sized> NewServiceExt for T where T: NewService {}

/// Trait for types that can be converted to a `Service`
pub trait IntoService<T>
where
    T: Service,
{
    /// Convert to a `Service`
    fn into_service(self) -> T;
}

/// Trait for types that can be converted to a Service
pub trait IntoNewService<T>
where
    T: NewService,
{
    /// Convert to an `NewService`
    fn into_new_service(self) -> T;
}

impl<T> IntoService<T> for T
where
    T: Service,
{
    fn into_service(self) -> T {
        self
    }
}

impl<T> IntoNewService<T> for T
where
    T: NewService,
{
    fn into_new_service(self) -> T {
        self
    }
}
