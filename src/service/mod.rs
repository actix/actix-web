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
        F: Into<B>,
        B: Service<Request = Self::Response, Error = Self::Error>,
    {
        AndThen::new(self, service.into())
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
        F: Into<B>,
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

impl<T: ?Sized> ServiceExt for T where T: Service {}
impl<T: ?Sized> NewServiceExt for T where T: NewService {}

impl<F, Req, Resp, Err, Fut> From<F> for FnService<F, Req, Resp, Err, Fut>
where
    F: Fn(Req) -> Fut + 'static,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    fn from(f: F) -> FnService<F, Req, Resp, Err, Fut> {
        FnService::new(f)
    }
}
