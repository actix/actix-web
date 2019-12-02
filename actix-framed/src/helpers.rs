use std::task::{Context, Poll};

use actix_http::Error;
use actix_service::{Service, ServiceFactory};
use futures::future::{FutureExt, LocalBoxFuture};

pub(crate) type BoxedHttpService<Req> = Box<
    dyn Service<
        Request = Req,
        Response = (),
        Error = Error,
        Future = LocalBoxFuture<'static, Result<(), Error>>,
    >,
>;

pub(crate) type BoxedHttpNewService<Req> = Box<
    dyn ServiceFactory<
        Config = (),
        Request = Req,
        Response = (),
        Error = Error,
        InitError = (),
        Service = BoxedHttpService<Req>,
        Future = LocalBoxFuture<'static, Result<BoxedHttpService<Req>, ()>>,
    >,
>;

pub(crate) struct HttpNewService<T: ServiceFactory>(T);

impl<T> HttpNewService<T>
where
    T: ServiceFactory<Response = (), Error = Error>,
    T::Response: 'static,
    T::Future: 'static,
    T::Service: Service<Future = LocalBoxFuture<'static, Result<(), Error>>> + 'static,
    <T::Service as Service>::Future: 'static,
{
    pub fn new(service: T) -> Self {
        HttpNewService(service)
    }
}

impl<T> ServiceFactory for HttpNewService<T>
where
    T: ServiceFactory<Config = (), Response = (), Error = Error>,
    T::Request: 'static,
    T::Future: 'static,
    T::Service: Service<Future = LocalBoxFuture<'static, Result<(), Error>>> + 'static,
    <T::Service as Service>::Future: 'static,
{
    type Config = ();
    type Request = T::Request;
    type Response = ();
    type Error = Error;
    type InitError = ();
    type Service = BoxedHttpService<T::Request>;
    type Future = LocalBoxFuture<'static, Result<Self::Service, ()>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let fut = self.0.new_service(());

        async move {
            fut.await.map_err(|_| ()).map(|service| {
                let service: BoxedHttpService<_> =
                    Box::new(HttpServiceWrapper { service });
                service
            })
        }
            .boxed_local()
    }
}

struct HttpServiceWrapper<T: Service> {
    service: T,
}

impl<T> Service for HttpServiceWrapper<T>
where
    T: Service<
        Response = (),
        Future = LocalBoxFuture<'static, Result<(), Error>>,
        Error = Error,
    >,
    T::Request: 'static,
{
    type Request = T::Request;
    type Response = ();
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<(), Error>>;

    fn poll_ready(&mut self, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        self.service.call(req)
    }
}
