use actix_http::Response;
use actix_service::{NewService, Service};
use futures::future::{ok, FutureResult};
use futures::{Future, Poll};

pub(crate) type BoxedHttpService<Req, Res> = Box<
    Service<
        Request = Req,
        Response = Res,
        Error = (),
        Future = Box<Future<Item = Res, Error = ()>>,
    >,
>;

pub(crate) type BoxedHttpNewService<Req, Res> = Box<
    NewService<
        Request = Req,
        Response = Res,
        Error = (),
        InitError = (),
        Service = BoxedHttpService<Req, Res>,
        Future = Box<Future<Item = BoxedHttpService<Req, Res>, Error = ()>>,
    >,
>;

pub(crate) struct HttpNewService<T: NewService>(T);

impl<T> HttpNewService<T>
where
    T: NewService,
    T::Response: 'static,
    T::Future: 'static,
    T::Service: Service,
    <T::Service as Service>::Future: 'static,
{
    pub fn new(service: T) -> Self {
        HttpNewService(service)
    }
}

impl<T> NewService for HttpNewService<T>
where
    T: NewService,
    T::Request: 'static,
    T::Response: 'static,
    T::Future: 'static,
    T::Service: Service + 'static,
    <T::Service as Service>::Future: 'static,
{
    type Request = T::Request;
    type Response = T::Response;
    type Error = ();
    type InitError = ();
    type Service = BoxedHttpService<T::Request, T::Response>;
    type Future = Box<Future<Item = Self::Service, Error = Self::InitError>>;

    fn new_service(&self, _: &()) -> Self::Future {
        Box::new(self.0.new_service(&()).map_err(|_| ()).and_then(|service| {
            let service: BoxedHttpService<_, _> =
                Box::new(HttpServiceWrapper { service });
            Ok(service)
        }))
    }
}

struct HttpServiceWrapper<T: Service> {
    service: T,
}

impl<T> Service for HttpServiceWrapper<T>
where
    T: Service,
    T::Request: 'static,
    T::Response: 'static,
    T::Future: 'static,
{
    type Request = T::Request;
    type Response = T::Response;
    type Error = ();
    type Future = Box<Future<Item = Self::Response, Error = Self::Error>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready().map_err(|_| ())
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        Box::new(self.service.call(req).map_err(|_| ()))
    }
}

pub(crate) fn not_found<Req>(_: Req) -> FutureResult<Response, ()> {
    ok(Response::NotFound().finish())
}

pub(crate) type HttpDefaultService<Req, Res> = Box<
    Service<
        Request = Req,
        Response = Res,
        Error = (),
        Future = Box<Future<Item = Res, Error = ()>>,
    >,
>;

pub(crate) type HttpDefaultNewService<Req, Res> = Box<
    NewService<
        Request = Req,
        Response = Res,
        Error = (),
        InitError = (),
        Service = HttpDefaultService<Req, Res>,
        Future = Box<Future<Item = HttpDefaultService<Req, Res>, Error = ()>>,
    >,
>;

pub(crate) struct DefaultNewService<T: NewService> {
    service: T,
}

impl<T> DefaultNewService<T>
where
    T: NewService + 'static,
    T::Future: 'static,
    <T::Service as Service>::Future: 'static,
{
    pub fn new(service: T) -> Self {
        DefaultNewService { service }
    }
}

impl<T> NewService for DefaultNewService<T>
where
    T: NewService + 'static,
    T::Request: 'static,
    T::Future: 'static,
    T::Service: 'static,
    <T::Service as Service>::Future: 'static,
{
    type Request = T::Request;
    type Response = T::Response;
    type Error = ();
    type InitError = ();
    type Service = HttpDefaultService<T::Request, T::Response>;
    type Future = Box<Future<Item = Self::Service, Error = Self::InitError>>;

    fn new_service(&self, _: &()) -> Self::Future {
        Box::new(
            self.service
                .new_service(&())
                .map_err(|_| ())
                .and_then(|service| {
                    let service: HttpDefaultService<_, _> =
                        Box::new(DefaultServiceWrapper { service });
                    Ok(service)
                }),
        )
    }
}

struct DefaultServiceWrapper<T: Service> {
    service: T,
}

impl<T> Service for DefaultServiceWrapper<T>
where
    T: Service + 'static,
    T::Future: 'static,
{
    type Request = T::Request;
    type Response = T::Response;
    type Error = ();
    type Future = Box<Future<Item = Self::Response, Error = Self::Error>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready().map_err(|_| ())
    }

    fn call(&mut self, req: T::Request) -> Self::Future {
        Box::new(self.service.call(req).map_err(|_| ()))
    }
}
