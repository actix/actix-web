use std::{
    convert::TryFrom,
    future::Future,
    net::SocketAddr,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use actix_http::{
    body::Body,
    client::{InvalidUrl, SendRequestError},
    http::{header, Method, StatusCode, Uri},
    RequestHead, RequestHeadType,
};
use actix_service::Service;
use bytes::Bytes;
use futures_core::ready;

use super::Transform;

use crate::connect::{ConnectRequest, ConnectResponse};
use crate::ClientResponse;

pub struct Redirect {
    max_redirect_times: u8,
}

impl Default for Redirect {
    fn default() -> Self {
        Self::new()
    }
}

impl Redirect {
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

impl<S> Transform<S, ConnectRequest> for Redirect
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

                // backup the uri and method for reuse schema and authority.
                let (uri, method) = match head {
                    RequestHeadType::Owned(ref head) => (head.uri.clone(), head.method.clone()),
                    RequestHeadType::Rc(ref head, ..) => {
                        (head.uri.clone(), head.method.clone())
                    }
                };

                let body_opt = match body {
                    Body::Bytes(ref b) => Some(b.clone()),
                    _ => None,
                };

                let fut = connector.call(ConnectRequest::Client(head, body, addr));

                RedirectServiceFuture::Client {
                    fut,
                    max_redirect_times,
                    uri: Some(uri),
                    method: Some(method),
                    body: body_opt,
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
            method: Option<Method>,
            body: Option<Bytes>,
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
                method,
                body,
                addr,
                connector,
            } => match ready!(fut.poll(cx))? {
                ConnectResponse::Client(res) => match res.head().status {
                    StatusCode::MOVED_PERMANENTLY
                    | StatusCode::FOUND
                    | StatusCode::SEE_OTHER
                        if *max_redirect_times > 0 =>
                    {
                        let org_uri = uri.take().unwrap();
                        // rebuild uri from the location header value.
                        let uri = rebuild_uri(&res, org_uri)?;

                        // reset method
                        let method = method.take().unwrap();
                        let method = match method {
                            Method::GET | Method::HEAD => method,
                            _ => Method::GET,
                        };

                        // take ownership of states that could be reused
                        let addr = addr.take();
                        let connector = connector.take();
                        let mut max_redirect_times = *max_redirect_times;

                        // use a new request head.
                        let mut head = RequestHead::default();
                        head.uri = uri.clone();
                        head.method = method.clone();

                        let head = RequestHeadType::Owned(head);

                        max_redirect_times -= 1;

                        let fut = connector
                            .as_ref()
                            .unwrap()
                            // remove body
                            .call(ConnectRequest::Client(head, Body::None, addr));

                        self.set(RedirectServiceFuture::Client {
                            fut,
                            max_redirect_times,
                            uri: Some(uri),
                            method: Some(method),
                            // body is dropped on 301,302,303
                            body: None,
                            addr,
                            connector,
                        });

                        self.poll(cx)
                    }
                    StatusCode::TEMPORARY_REDIRECT | StatusCode::PERMANENT_REDIRECT
                        if *max_redirect_times > 0 =>
                    {
                        let org_uri = uri.take().unwrap();
                        // rebuild uri from the location header value.
                        let uri = rebuild_uri(&res, org_uri)?;

                        // try to reuse body
                        let body = body.take();
                        let body_new = match body {
                            Some(ref bytes) => Body::Bytes(bytes.clone()),
                            // TODO: should this be Body::Empty or Body::None.
                            _ => Body::Empty,
                        };

                        let addr = addr.take();
                        let method = method.take().unwrap();
                        let connector = connector.take();
                        let mut max_redirect_times = *max_redirect_times;

                        // use a new request head.
                        let mut head = RequestHead::default();
                        head.uri = uri.clone();
                        head.method = method.clone();

                        let head = RequestHeadType::Owned(head);

                        max_redirect_times -= 1;

                        let fut = connector
                            .as_ref()
                            .unwrap()
                            .call(ConnectRequest::Client(head, body_new, addr));

                        self.set(RedirectServiceFuture::Client {
                            fut,
                            max_redirect_times,
                            uri: Some(uri),
                            method: Some(method),
                            body,
                            addr,
                            connector,
                        });

                        self.poll(cx)
                    }
                    _ => Poll::Ready(Ok(ConnectResponse::Client(res))),
                },
                _ => unreachable!("ConnectRequest::Tunnel is not handled by Redirect"),
            },
        }
    }
}

fn rebuild_uri(res: &ClientResponse, org_uri: Uri) -> Result<Uri, SendRequestError> {
    let uri = res
        .headers()
        .get(header::LOCATION)
        .map(|value| {
            // try to parse the location to a full uri
            let uri = Uri::try_from(value.as_bytes())
                .map_err(|e| SendRequestError::Url(InvalidUrl::HttpError(e.into())))?;
            if uri.scheme().is_none() || uri.authority().is_none() {
                let uri = Uri::builder()
                    .scheme(org_uri.scheme().cloned().unwrap())
                    .authority(org_uri.authority().cloned().unwrap())
                    .path_and_query(value.as_bytes())
                    .build()?;
                Ok::<_, SendRequestError>(uri)
            } else {
                Ok(uri)
            }
        })
        // TODO: this error type is wrong.
        .ok_or(SendRequestError::Url(InvalidUrl::MissingScheme))??;

    Ok(uri)
}

#[cfg(test)]
mod tests {
    use actix_web::{web, App, Error, HttpResponse};

    use super::*;
    use crate::ClientBuilder;

    #[actix_rt::test]
    async fn test_basic_redirect() {
        let client = ClientBuilder::new()
            .disable_redirects()
            .wrap(Redirect::new().max_redirect_times(10))
            .finish();

        let srv = actix_test::start(|| {
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
            .disable_redirects()
            .wrap(Redirect::new().max_redirect_times(1))
            .connector(crate::Connector::new())
            .finish();

        let srv = actix_test::start(|| {
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
