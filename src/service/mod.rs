use futures::{Future, IntoFuture};

mod and_then;
mod apply;
mod fn_service;
mod fn_state_service;
mod from_err;
mod map;
mod map_err;
mod map_init_err;

pub use self::and_then::{AndThen, AndThenNewService};
pub use self::apply::{Apply, ApplyNewService};
pub use self::fn_service::{FnNewService, FnService};
pub use self::fn_state_service::{FnStateNewService, FnStateService};
pub use self::from_err::FromErr;
pub use self::map::{Map, MapNewService};
pub use self::map_err::{MapErr, MapErrNewService};
pub use self::map_init_err::MapInitErr;
use {NewService, Service};

pub trait ServiceExt: Service {
    fn apply<F, R, Req>(self, f: F) -> Apply<Self, F, R, Req>
    where
        Self: Sized,
        Self::Error: Into<<R::Future as Future>::Error>,
        F: Fn(Req, &mut Self) -> R,
        R: IntoFuture,
    {
        Apply::new(f, self)
    }

    fn and_then<F, B>(self, service: F) -> AndThen<Self, B>
    where
        Self: Sized,
        F: IntoService<B>,
        B: Service<Request = Self::Response, Error = Self::Error>,
    {
        AndThen::new(self, service.into_service())
    }

    fn from_err<E>(self) -> FromErr<Self, E>
    where
        Self: Sized,
        E: From<Self::Error>,
    {
        FromErr::new(self)
    }

    fn map<F, R>(self, f: F) -> Map<Self, F, R>
    where
        Self: Sized,
        F: Fn(Self::Response) -> R,
    {
        Map::new(self, f)
    }

    fn map_err<F, E>(self, f: F) -> MapErr<Self, F, E>
    where
        Self: Sized,
        F: Fn(Self::Error) -> E,
    {
        MapErr::new(self, f)
    }
}

pub trait NewServiceExt: NewService {
    fn apply<F, R, Req>(self, f: F) -> ApplyNewService<Self, F, R, Req>
    where
        Self: Sized,
        Self::Error: Into<<R::Future as Future>::Error>,
        F: Fn(Req, &mut Self::Service) -> R + Clone,
        R: IntoFuture,
    {
        ApplyNewService::new(f, self)
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
