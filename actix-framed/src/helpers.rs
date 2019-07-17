use actix_http::Error;
use actix_service::{NewService, Service};
use futures::{Future, Poll};

pub(crate) type BoxedHttpService<Req> = Box<
    dyn Service<
        Request = Req,
        Response = (),
        Error = Error,
        Future = Box<dyn Future<Item = (), Error = Error>>,
    >,
>;

pub(crate) type BoxedHttpNewService<Req> = Box<
    dyn NewService<
        Config = (),
        Request = Req,
        Response = (),
        Error = Error,
        InitError = (),
        Service = BoxedHttpService<Req>,
        Future = Box<dyn Future<Item = BoxedHttpService<Req>, Error = ()>>,
    >,
>;

pub(crate) struct HttpNewService<T: NewService>(T);

impl<T> HttpNewService<T>
where
    T: NewService<Response = (), Error = Error>,
    T::Response: 'static,
    T::Future: 'static,
    T::Service: Service<Future = Box<dyn Future<Item = (), Error = Error>>> + 'static,
    <T::Service as Service>::Future: 'static,
{
    pub fn new(service: T) -> Self {
        HttpNewService(service)
    }
}

impl<T> NewService for HttpNewService<T>
where
    T: NewService<Config = (), Response = (), Error = Error>,
    T::Request: 'static,
    T::Future: 'static,
    T::Service: Service<Future = Box<dyn Future<Item = (), Error = Error>>> + 'static,
    <T::Service as Service>::Future: 'static,
{
    type Config = ();
    type Request = T::Request;
    type Response = ();
    type Error = Error;
    type InitError = ();
    type Service = BoxedHttpService<T::Request>;
    type Future = Box<dyn Future<Item = Self::Service, Error = ()>>;

    fn new_service(&self, _: &()) -> Self::Future {
        Box::new(self.0.new_service(&()).map_err(|_| ()).and_then(|service| {
            let service: BoxedHttpService<_> = Box::new(HttpServiceWrapper { service });
            Ok(service)
        }))
    }
}

struct HttpServiceWrapper<T: Service> {
    service: T,
}

impl<T> Service for HttpServiceWrapper<T>
where
    T: Service<
        Response = (),
        Future = Box<dyn Future<Item = (), Error = Error>>,
        Error = Error,
    >,
    T::Request: 'static,
{
    type Request = T::Request;
    type Response = ();
    type Error = Error;
    type Future = Box<dyn Future<Item = (), Error = Error>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready()
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        self.service.call(req)
    }
}
