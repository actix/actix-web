use std::rc::Rc;

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_http::h1::{Codec, SendResponse};
use actix_http::{Error, Request, Response};
use actix_router::{Path, Router, Url};
use actix_server_config::ServerConfig;
use actix_service::{IntoNewService, NewService, Service};
use futures::{Async, Future, Poll};

use crate::helpers::{BoxedHttpNewService, BoxedHttpService, HttpNewService};
use crate::request::FramedRequest;
use crate::state::State;

type BoxedResponse = Box<dyn Future<Item = (), Error = Error>>;

pub trait HttpServiceFactory {
    type Factory: NewService;

    fn path(&self) -> &str;

    fn create(self) -> Self::Factory;
}

/// Application builder
pub struct FramedApp<T, S = ()> {
    state: State<S>,
    services: Vec<(String, BoxedHttpNewService<FramedRequest<T, S>>)>,
}

impl<T: 'static> FramedApp<T, ()> {
    pub fn new() -> Self {
        FramedApp {
            state: State::new(()),
            services: Vec::new(),
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
        U: HttpServiceFactory,
        U::Factory: NewService<
                Config = (),
                Request = FramedRequest<T, S>,
                Response = (),
                Error = Error,
                InitError = (),
            > + 'static,
        <U::Factory as NewService>::Future: 'static,
        <U::Factory as NewService>::Service: Service<
            Request = FramedRequest<T, S>,
            Response = (),
            Error = Error,
            Future = Box<dyn Future<Item = (), Error = Error>>,
        >,
    {
        let path = factory.path().to_string();
        self.services
            .push((path, Box::new(HttpNewService::new(factory.create()))));
        self
    }
}

impl<T, S> IntoNewService<FramedAppFactory<T, S>> for FramedApp<T, S>
where
    T: AsyncRead + AsyncWrite + 'static,
    S: 'static,
{
    fn into_new_service(self) -> FramedAppFactory<T, S> {
        FramedAppFactory {
            state: self.state,
            services: Rc::new(self.services),
        }
    }
}

#[derive(Clone)]
pub struct FramedAppFactory<T, S> {
    state: State<S>,
    services: Rc<Vec<(String, BoxedHttpNewService<FramedRequest<T, S>>)>>,
}

impl<T, S> NewService for FramedAppFactory<T, S>
where
    T: AsyncRead + AsyncWrite + 'static,
    S: 'static,
{
    type Config = ServerConfig;
    type Request = (Request, Framed<T, Codec>);
    type Response = ();
    type Error = Error;
    type InitError = ();
    type Service = FramedAppService<T, S>;
    type Future = CreateService<T, S>;

    fn new_service(&self, _: &ServerConfig) -> Self::Future {
        CreateService {
            fut: self
                .services
                .iter()
                .map(|(path, service)| {
                    CreateServiceItem::Future(
                        Some(path.clone()),
                        service.new_service(&()),
                    )
                })
                .collect(),
            state: self.state.clone(),
        }
    }
}

#[doc(hidden)]
pub struct CreateService<T, S> {
    fut: Vec<CreateServiceItem<T, S>>,
    state: State<S>,
}

enum CreateServiceItem<T, S> {
    Future(
        Option<String>,
        Box<dyn Future<Item = BoxedHttpService<FramedRequest<T, S>>, Error = ()>>,
    ),
    Service(String, BoxedHttpService<FramedRequest<T, S>>),
}

impl<S: 'static, T: 'static> Future for CreateService<T, S>
where
    T: AsyncRead + AsyncWrite,
{
    type Item = FramedAppService<T, S>;
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
                            router.path(&path, service);
                        }
                        CreateServiceItem::Future(_, _) => unreachable!(),
                    }
                    router
                });
            Ok(Async::Ready(FramedAppService {
                router: router.finish(),
                state: self.state.clone(),
            }))
        } else {
            Ok(Async::NotReady)
        }
    }
}

pub struct FramedAppService<T, S> {
    state: State<S>,
    router: Router<BoxedHttpService<FramedRequest<T, S>>>,
}

impl<S: 'static, T: 'static> Service for FramedAppService<T, S>
where
    T: AsyncRead + AsyncWrite,
{
    type Request = (Request, Framed<T, Codec>);
    type Response = ();
    type Error = Error;
    type Future = BoxedResponse;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, (req, framed): (Request, Framed<T, Codec>)) -> Self::Future {
        let mut path = Path::new(Url::new(req.uri().clone()));

        if let Some((srv, _info)) = self.router.recognize_mut(&mut path) {
            return srv.call(FramedRequest::new(req, framed, path, self.state.clone()));
        }
        Box::new(
            SendResponse::new(framed, Response::NotFound().finish()).then(|_| Ok(())),
        )
    }
}
