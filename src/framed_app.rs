use std::marker::PhantomData;
use std::rc::Rc;

use actix_codec::Framed;
use actix_http::h1::Codec;
use actix_http::{Request, Response, SendResponse};
use actix_router::{Path, Router, Url};
use actix_service::{IntoNewService, NewService, Service};
use actix_utils::cloneable::CloneableService;
use futures::{Async, Future, Poll};
use tokio_io::{AsyncRead, AsyncWrite};

use crate::app::{HttpServiceFactory, State};
use crate::framed_handler::FramedRequest;
use crate::helpers::{BoxedHttpNewService, BoxedHttpService, HttpNewService};
use crate::request::Request as WebRequest;

pub type FRequest<T> = (Request, Framed<T, Codec>);
type BoxedResponse = Box<Future<Item = (), Error = ()>>;

/// Application builder
pub struct FramedApp<T, S = ()> {
    services: Vec<(String, BoxedHttpNewService<FramedRequest<S, T>, ()>)>,
    state: State<S>,
}

impl<T: 'static> FramedApp<T, ()> {
    pub fn new() -> Self {
        FramedApp {
            services: Vec::new(),
            state: State::new(()),
        }
    }
}

impl<T: 'static, S: 'static> FramedApp<T, S> {
    pub fn with(state: S) -> FramedApp<T, S> {
        FramedApp {
            services: Vec::new(),
            state: State::new(state),
        }
    }

    pub fn service<U>(mut self, factory: U) -> Self
    where
        U: HttpServiceFactory<S>,
        U::Factory: NewService<Request = FramedRequest<S, T>, Response = ()> + 'static,
        <U::Factory as NewService>::Future: 'static,
        <U::Factory as NewService>::Service: Service<Request = FramedRequest<S, T>>,
        <<U::Factory as NewService>::Service as Service>::Future: 'static,
    {
        let path = factory.path().to_string();
        self.services.push((
            path,
            Box::new(HttpNewService::new(factory.create(self.state.clone()))),
        ));
        self
    }

    pub fn register_service<U>(&mut self, factory: U)
    where
        U: HttpServiceFactory<S>,
        U::Factory: NewService<Request = FramedRequest<S, T>, Response = ()> + 'static,
        <U::Factory as NewService>::Future: 'static,
        <U::Factory as NewService>::Service: Service<Request = FramedRequest<S, T>>,
        <<U::Factory as NewService>::Service as Service>::Future: 'static,
    {
        let path = factory.path().to_string();
        self.services.push((
            path,
            Box::new(HttpNewService::new(factory.create(self.state.clone()))),
        ));
    }
}

impl<T: 'static, S: 'static> IntoNewService<FramedAppFactory<S, T>> for FramedApp<T, S>
where
    T: AsyncRead + AsyncWrite,
{
    fn into_new_service(self) -> FramedAppFactory<S, T> {
        FramedAppFactory {
            state: self.state,
            services: Rc::new(self.services),
            _t: PhantomData,
        }
    }
}

#[derive(Clone)]
pub struct FramedAppFactory<S, T> {
    state: State<S>,
    services: Rc<Vec<(String, BoxedHttpNewService<FramedRequest<S, T>, ()>)>>,
    _t: PhantomData<T>,
}

impl<S: 'static, T: 'static> NewService for FramedAppFactory<S, T>
where
    T: AsyncRead + AsyncWrite,
{
    type Request = FRequest<T>;
    type Response = ();
    type Error = ();
    type InitError = ();
    type Service = CloneableService<FramedAppService<S, T>>;
    type Future = CreateService<S, T>;

    fn new_service(&self) -> Self::Future {
        CreateService {
            fut: self
                .services
                .iter()
                .map(|(path, service)| {
                    CreateServiceItem::Future(Some(path.clone()), service.new_service())
                })
                .collect(),
            state: self.state.clone(),
        }
    }
}

#[doc(hidden)]
pub struct CreateService<S, T> {
    fut: Vec<CreateServiceItem<S, T>>,
    state: State<S>,
}

enum CreateServiceItem<S, T> {
    Future(
        Option<String>,
        Box<Future<Item = BoxedHttpService<FramedRequest<S, T>, ()>, Error = ()>>,
    ),
    Service(String, BoxedHttpService<FramedRequest<S, T>, ()>),
}

impl<S: 'static, T: 'static> Future for CreateService<S, T>
where
    T: AsyncRead + AsyncWrite,
{
    type Item = CloneableService<FramedAppService<S, T>>;
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let mut done = true;

        // poll http services
        for item in &mut self.fut {
            let res = match item {
                CreateServiceItem::Future(ref mut path, ref mut fut) => {
                    match fut.poll()? {
                        Async::Ready(service) => Some((path.take().unwrap(), service)),
                        Async::NotReady => {
                            done = false;
                            None
                        }
                    }
                }
                CreateServiceItem::Service(_, _) => continue,
            };

            if let Some((path, service)) = res {
                *item = CreateServiceItem::Service(path, service);
            }
        }

        if done {
            let router = self
                .fut
                .drain(..)
                .fold(Router::build(), |mut router, item| {
                    match item {
                        CreateServiceItem::Service(path, service) => {
                            router.path(&path, service)
                        }
                        CreateServiceItem::Future(_, _) => unreachable!(),
                    }
                    router
                });
            Ok(Async::Ready(CloneableService::new(FramedAppService {
                router: router.finish(),
                state: self.state.clone(),
                // default: self.default.take().expect("something is wrong"),
            })))
        } else {
            Ok(Async::NotReady)
        }
    }
}

pub struct FramedAppService<S, T> {
    state: State<S>,
    router: Router<BoxedHttpService<FramedRequest<S, T>, ()>>,
}

impl<S: 'static, T: 'static> Service for FramedAppService<S, T>
where
    T: AsyncRead + AsyncWrite,
{
    type Request = FRequest<T>;
    type Response = ();
    type Error = ();
    type Future = BoxedResponse;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        // let mut ready = true;
        // for service in &mut self.services {
        //     if let Async::NotReady = service.poll_ready()? {
        //         ready = false;
        //     }
        // }
        // if ready {
        //     Ok(Async::Ready(()))
        // } else {
        //     Ok(Async::NotReady)
        // }
        Ok(Async::Ready(()))
    }

    fn call(&mut self, (req, framed): (Request, Framed<T, Codec>)) -> Self::Future {
        let mut path = Path::new(Url::new(req.uri().clone()));

        if let Some((srv, _info)) = self.router.recognize_mut(&mut path) {
            return srv.call(FramedRequest::new(
                WebRequest::new(self.state.clone(), req, path),
                framed,
            ));
        }
        // for item in &mut self.services {
        //     req = match item.handle(req) {
        //         Ok(fut) => return fut,
        //         Err(req) => req,
        //     };
        // }
        // self.default.call(req)
        Box::new(
            SendResponse::send(framed, Response::NotFound().finish().into())
                .map(|_| ())
                .map_err(|_| ()),
        )
    }
}
