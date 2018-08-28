use futures::IntoFuture;
use tower_service::{NewService, Service};

mod and_then;
mod fn_service;
mod fn_state_service;
mod map;
mod map_err;
mod map_init_err;

pub use self::and_then::{AndThen, AndThenNewService};
pub use self::fn_service::FnService;
pub use self::fn_state_service::FnStateService;
pub use self::map::{Map, MapNewService};
pub use self::map_err::{MapErr, MapErrNewService};
pub use self::map_init_err::MapInitErr;

pub trait NewServiceExt: NewService {
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

impl<T: NewService> NewServiceExt for T {}

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
    fn into_new_service(self) -> T;
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
    fn into_new_service(self) -> T {
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
