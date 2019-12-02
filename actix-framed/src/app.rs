use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_http::h1::{Codec, SendResponse};
use actix_http::{Error, Request, Response};
use actix_router::{Path, Router, Url};
use actix_service::{IntoServiceFactory, Service, ServiceFactory};
use futures::future::{ok, FutureExt, LocalBoxFuture};

use crate::helpers::{BoxedHttpNewService, BoxedHttpService, HttpNewService};
use crate::request::FramedRequest;
use crate::state::State;

type BoxedResponse = LocalBoxFuture<'static, Result<(), Error>>;

pub trait HttpServiceFactory {
    type Factory: ServiceFactory;

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
        U::Factory: ServiceFactory<
                Config = (),
                Request = FramedRequest<T, S>,
                Response = (),
                Error = Error,
                InitError = (),
            > + 'static,
        <U::Factory as ServiceFactory>::Future: 'static,
        <U::Factory as ServiceFactory>::Service: Service<
            Request = FramedRequest<T, S>,
            Response = (),
            Error = Error,
            Future = LocalBoxFuture<'static, Result<(), Error>>,
        >,
    {
        let path = factory.path().to_string();
        self.services
            .push((path, Box::new(HttpNewService::new(factory.create()))));
        self
    }
}

impl<T, S> IntoServiceFactory<FramedAppFactory<T, S>> for FramedApp<T, S>
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
    S: 'static,
{
    fn into_factory(self) -> FramedAppFactory<T, S> {
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

impl<T, S> ServiceFactory for FramedAppFactory<T, S>
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
    S: 'static,
{
    type Config = ();
    type Request = (Request, Framed<T, Codec>);
    type Response = ();
    type Error = Error;
    type InitError = ();
    type Service = FramedAppService<T, S>;
    type Future = CreateService<T, S>;

    fn new_service(&self, _: ()) -> Self::Future {
        CreateService {
            fut: self
                .services
                .iter()
                .map(|(path, service)| {
                    CreateServiceItem::Future(
                        Some(path.clone()),
                        service.new_service(()),
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
        LocalBoxFuture<'static, Result<BoxedHttpService<FramedRequest<T, S>>, ()>>,
    ),
    Service(String, BoxedHttpService<FramedRequest<T, S>>),
}

impl<S: 'static, T: 'static> Future for CreateService<T, S>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    type Output = Result<FramedAppService<T, S>, ()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let mut done = true;

        // poll http services
        for item in &mut self.fut {
            let res = match item {
                CreateServiceItem::Future(ref mut path, ref mut fut) => {
                    match Pin::new(fut).poll(cx) {
                        Poll::Ready(Ok(service)) => {
                            Some((path.take().unwrap(), service))
                        }
                        Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                        Poll::Pending => {
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
            Poll::Ready(Ok(FramedAppService {
                router: router.finish(),
                state: self.state.clone(),
            }))
        } else {
            Poll::Pending
        }
    }
}

pub struct FramedAppService<T, S> {
    state: State<S>,
    router: Router<BoxedHttpService<FramedRequest<T, S>>>,
}

impl<S: 'static, T: 'static> Service for FramedAppService<T, S>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    type Request = (Request, Framed<T, Codec>);
    type Response = ();
    type Error = Error;
    type Future = BoxedResponse;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, (req, framed): (Request, Framed<T, Codec>)) -> Self::Future {
        let mut path = Path::new(Url::new(req.uri().clone()));

        if let Some((srv, _info)) = self.router.recognize_mut(&mut path) {
            return srv.call(FramedRequest::new(req, framed, path, self.state.clone()));
        }
        SendResponse::new(framed, Response::NotFound().finish())
            .then(|_| ok(()))
            .boxed_local()
    }
}
