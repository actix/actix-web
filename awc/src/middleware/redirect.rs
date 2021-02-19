use std::{
    future::Future,
    net::SocketAddr,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use actix_http::{
    body::Body,
    client::{InvalidUrl, SendRequestError},
    http::{header, StatusCode, Uri},
    RequestHead, RequestHeadType,
};
use actix_service::Service;
use futures_core::ready;

use super::Transform;

use crate::connect::{ConnectRequest, ConnectResponse};

pub struct RedirectMiddleware {
    max_redirect_times: u8,
}

impl Default for RedirectMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

impl RedirectMiddleware {
    pub fn new() -> Self {
        Self {
            max_redirect_times: 10,
        }
    }

    pub fn max_redirect_times(mut self, times: u8) -> Self {
        self.max_redirect_times = times;
        self
    }
}

impl<S> Transform<S, ConnectRequest> for RedirectMiddleware
where
    S: Service<ConnectRequest, Response = ConnectResponse, Error = SendRequestError> + 'static,
{
    type Transform = RedirectService<S>;

    fn new_transform(self, service: S) -> Self::Transform {
        RedirectService {
            max_redirect_times: self.max_redirect_times,
            connector: Rc::new(service),
        }
    }
}

pub struct RedirectService<S> {
    max_redirect_times: u8,
    connector: Rc<S>,
}

impl<S> Service<ConnectRequest> for RedirectService<S>
where
    S: Service<ConnectRequest, Response = ConnectResponse, Error = SendRequestError> + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = RedirectServiceFuture<S>;

    actix_service::forward_ready!(connector);

    fn call(&self, req: ConnectRequest) -> Self::Future {
        match req {
            ConnectRequest::Tunnel(head, addr) => {
                let fut = self.connector.call(ConnectRequest::Tunnel(head, addr));
                RedirectServiceFuture::Tunnel { fut }
            }
            ConnectRequest::Client(head, body, addr) => {
                let connector = self.connector.clone();
                let max_redirect_times = self.max_redirect_times;

                // backup the uri for reuse schema and authority.
                let uri = match head {
                    RequestHeadType::Owned(ref head) => head.uri.clone(),
                    RequestHeadType::Rc(ref head, ..) => head.uri.clone(),
                };

                let fut = connector.call(ConnectRequest::Client(head, body, addr));

                RedirectServiceFuture::Client {
                    fut,
                    max_redirect_times,
                    uri: Some(uri),
                    addr,
                    connector: Some(connector),
                }
            }
        }
    }
}

pin_project_lite::pin_project! {
    #[project = RedirectServiceProj]
    pub enum RedirectServiceFuture<S>
    where
        S: Service<ConnectRequest, Response = ConnectResponse, Error = SendRequestError>,
        S: 'static
    {
        Tunnel { #[pin] fut: S::Future },
        Client {
            #[pin]
            fut: S::Future,
            max_redirect_times: u8,
            uri: Option<Uri>,
            addr: Option<SocketAddr>,
            connector: Option<Rc<S>>
        }
    }
}

impl<S> Future for RedirectServiceFuture<S>
where
    S: Service<ConnectRequest, Response = ConnectResponse, Error = SendRequestError> + 'static,
{
    type Output = Result<ConnectResponse, SendRequestError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.as_mut().project() {
            RedirectServiceProj::Tunnel { fut } => fut.poll(cx),
            RedirectServiceProj::Client {
                fut,
                max_redirect_times,
                uri,
                addr,
                connector,
            } => {
                match ready!(fut.poll(cx))? {
                    ConnectResponse::Client(res) => match res.head().status {
                        StatusCode::MOVED_PERMANENTLY
                        | StatusCode::FOUND
                        | StatusCode::SEE_OTHER
                        | StatusCode::TEMPORARY_REDIRECT
                        | StatusCode::PERMANENT_REDIRECT
                            if *max_redirect_times > 0 =>
                        {
                            let uri = uri.take().unwrap();

                            // rebuild uri from the location header value.
                            let uri = res
                                .headers()
                                .get(header::LOCATION)
                                .map(|value| {
                                    Uri::builder()
                                        .scheme(uri.scheme().cloned().unwrap())
                                        .authority(uri.authority().cloned().unwrap())
                                        .path_and_query(value.as_bytes())
                                })
                                .ok_or(SendRequestError::Url(InvalidUrl::MissingScheme))?
                                .build()?;

                            let addr = addr.take();
                            let connector = connector.take();
                            let mut max_redirect_times = *max_redirect_times;

                            // use a new request head.
                            let mut head = RequestHead::default();
                            head.uri = uri.clone();
                            let head = RequestHeadType::Owned(head);

                            // throw body
                            let body = Body::None;

                            max_redirect_times -= 1;

                            let fut = connector
                                .as_ref()
                                .unwrap()
                                .call(ConnectRequest::Client(head, body, addr));

                            self.as_mut().set(RedirectServiceFuture::Client {
                                fut,
                                max_redirect_times,
                                uri: Some(uri),
                                addr,
                                connector,
                            });

                            self.poll(cx)
                        }
                        _ => Poll::Ready(Ok(ConnectResponse::Client(res))),
                    },
                    _ => unreachable!("ConnectRequest::Tunnel is not handled by Redirect"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use actix_web::{test::start, web, App, Error, HttpResponse};

    use super::*;

    use crate::ClientBuilder;

    #[actix_rt::test]
    async fn test_basic_redirect() {
        let client = ClientBuilder::new()
            .connector(crate::Connector::new())
            .wrap(RedirectMiddleware::new().max_redirect_times(10))
            .finish();

        let srv = start(|| {
            App::new()
                .service(web::resource("/test").route(web::to(|| async {
                    Ok::<_, Error>(HttpResponse::BadRequest())
                })))
                .service(web::resource("/").route(web::to(|| async {
                    Ok::<_, Error>(
                        HttpResponse::Found()
                            .append_header(("location", "/test"))
                            .finish(),
                    )
                })))
        });

        let res = client.get(srv.url("/")).send().await.unwrap();

        assert_eq!(res.status().as_u16(), 400);
    }

    #[actix_rt::test]
    async fn test_redirect_limit() {
        let client = ClientBuilder::new()
            .wrap(RedirectMiddleware::new().max_redirect_times(1))
            .connector(crate::Connector::new())
            .finish();

        let srv = start(|| {
            App::new()
                .service(web::resource("/").route(web::to(|| async {
                    Ok::<_, Error>(
                        HttpResponse::Found()
                            .append_header(("location", "/test"))
                            .finish(),
                    )
                })))
                .service(web::resource("/test").route(web::to(|| async {
                    Ok::<_, Error>(
                        HttpResponse::Found()
                            .append_header(("location", "/test2"))
                            .finish(),
                    )
                })))
                .service(web::resource("/test2").route(web::to(|| async {
                    Ok::<_, Error>(HttpResponse::BadRequest())
                })))
        });

        let res = client.get(srv.url("/")).send().await.unwrap();

        assert_eq!(res.status().as_u16(), 302);
    }
}
