use futures::{Future, IntoFuture};

mod and_then;
mod apply;
mod fn_service;
mod fn_state_service;
mod map;
mod map_err;
mod map_init_err;
mod map_request;

pub use self::and_then::{AndThen, AndThenNewService};
pub use self::apply::{Apply, ApplyNewService};
pub use self::fn_service::{FnNewService, FnService};
pub use self::fn_state_service::{FnStateNewService, FnStateService};
pub use self::map::{Map, MapNewService};
pub use self::map_err::{MapErr, MapErrNewService};
pub use self::map_init_err::MapInitErr;
pub use self::map_request::{MapReq, MapReqNewService};
use {NewService, Service};

pub trait ServiceExt: Service {
    fn and_then<F, B>(self, new_service: F) -> AndThen<Self, B>
    where
        Self: Sized,
        F: IntoService<B>,
        B: Service<Request = Self::Response, Error = Self::Error>,
    {
        AndThen::new(self, new_service.into_service())
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
    fn apply<T, F, R, Req, Resp, Err, F2>(
        f: F, service: F2,
    ) -> ApplyNewService<T, F, R, Req, Resp, Err>
    where
        T: NewService,
        F: Fn(Req, &mut T::Service) -> R + Clone,
        R: Future<Item = Resp, Error = Err>,
        F2: IntoNewService<T>,
    {
        ApplyNewService::new(f, service.into_new_service())
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

    fn map_request<F, R>(self, f: F) -> MapReqNewService<Self, F, R>
    where
        Self: Sized,
        F: Fn(R) -> Self::Request,
    {
        MapReqNewService::new(self, f)
    }

    fn map_init_err<F, E>(self, f: F) -> MapInitErr<Self, F, E>
    where
        Self: Sized,
        F: Fn(Self::InitError) -> E,
    {
        MapInitErr::new(self, f)
    }
}

impl<T: Service> ServiceExt for T {}
impl<T: NewService> NewServiceExt for T {}

/// Trait for types that can be converted to a Service
pub trait IntoService<T>
where
    T: Service,
{
    /// Create service
    fn into_service(self) -> T;
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

impl<F, Req, Resp, Err, Fut> IntoService<FnService<F, Req, Resp, Err, Fut>> for F
where
    F: Fn(Req) -> Fut + 'static,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    fn into_service(self) -> FnService<F, Req, Resp, Err, Fut> {
        FnService::new(self)
    }
}
