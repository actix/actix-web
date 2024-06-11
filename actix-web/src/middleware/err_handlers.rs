//! For middleware documentation, see [`ErrorHandlers`].

use std::{
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use actix_service::{Service, Transform};
use ahash::AHashMap;
use futures_core::{future::LocalBoxFuture, ready};
use pin_project_lite::pin_project;

use crate::{
    body::EitherBody,
    dev::{ServiceRequest, ServiceResponse},
    http::StatusCode,
    Error, Result,
};

/// Return type for [`ErrorHandlers`] custom handlers.
pub enum ErrorHandlerResponse<B> {
    /// Immediate HTTP response.
    Response(ServiceResponse<EitherBody<B>>),

    /// A future that resolves to an HTTP response.
    Future(LocalBoxFuture<'static, Result<ServiceResponse<EitherBody<B>>, Error>>),
}

type ErrorHandler<B> = dyn Fn(ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>>;

type DefaultHandler<B> = Option<Rc<ErrorHandler<B>>>;

/// Middleware for registering custom status code based error handlers.
///
/// Register handlers with the [`ErrorHandlers::handler()`] method to register a custom error handler
/// for a given status code. Handlers can modify existing responses or create completely new ones.
///
/// To register a default handler, use the [`ErrorHandlers::default_handler()`] method. This
/// handler will be used only if a response has an error status code (400-599) that isn't covered by
/// a more specific handler (set with the [`handler()`][ErrorHandlers::handler] method). See examples
/// below.
///
/// To register a default for only client errors (400-499) or only server errors (500-599), use the
/// [`ErrorHandlers::default_handler_client()`] and [`ErrorHandlers::default_handler_server()`]
/// methods, respectively.
///
/// Any response with a status code that isn't covered by a specific handler or a default handler
/// will pass by unchanged by this middleware.
///
/// # Examples
///
/// Adding a header:
///
/// ```
/// use actix_web::{
///     dev::ServiceResponse,
///     http::{header, StatusCode},
///     middleware::{ErrorHandlerResponse, ErrorHandlers},
///     web, App, HttpResponse, Result,
/// };
///
/// fn add_error_header<B>(mut res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
///     res.response_mut().headers_mut().insert(
///         header::CONTENT_TYPE,
///         header::HeaderValue::from_static("Error"),
///     );
///
///     // body is unchanged, map to "left" slot
///     Ok(ErrorHandlerResponse::Response(res.map_into_left_body()))
/// }
///
/// let app = App::new()
///     .wrap(ErrorHandlers::new().handler(StatusCode::INTERNAL_SERVER_ERROR, add_error_header))
///     .service(web::resource("/").route(web::get().to(HttpResponse::InternalServerError)));
/// ```
///
/// Modifying response body:
///
/// ```
/// use actix_web::{
///     dev::ServiceResponse,
///     http::{header, StatusCode},
///     middleware::{ErrorHandlerResponse, ErrorHandlers},
///     web, App, HttpResponse, Result,
/// };
///
/// fn add_error_body<B>(res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
///     // split service response into request and response components
///     let (req, res) = res.into_parts();
///
///     // set body of response to modified body
///     let res = res.set_body("An error occurred.");
///
///     // modified bodies need to be boxed and placed in the "right" slot
///     let res = ServiceResponse::new(req, res)
///         .map_into_boxed_body()
///         .map_into_right_body();
///
///     Ok(ErrorHandlerResponse::Response(res))
/// }
///
/// let app = App::new()
///     .wrap(ErrorHandlers::new().handler(StatusCode::INTERNAL_SERVER_ERROR, add_error_body))
///     .service(web::resource("/").route(web::get().to(HttpResponse::InternalServerError)));
/// ```
///
/// Registering default handler:
///
/// ```
/// # use actix_web::{
/// #     dev::ServiceResponse,
/// #     http::{header, StatusCode},
/// #     middleware::{ErrorHandlerResponse, ErrorHandlers},
/// #     web, App, HttpResponse, Result,
/// # };
/// fn add_error_header<B>(mut res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
///     res.response_mut().headers_mut().insert(
///         header::CONTENT_TYPE,
///         header::HeaderValue::from_static("Error"),
///     );
///
///     // body is unchanged, map to "left" slot
///     Ok(ErrorHandlerResponse::Response(res.map_into_left_body()))
/// }
///
/// fn handle_bad_request<B>(mut res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
///     res.response_mut().headers_mut().insert(
///         header::CONTENT_TYPE,
///         header::HeaderValue::from_static("Bad Request Error"),
///     );
///
///     // body is unchanged, map to "left" slot
///     Ok(ErrorHandlerResponse::Response(res.map_into_left_body()))
/// }
///
/// // Bad Request errors will hit `handle_bad_request()`, while all other errors will hit
/// // `add_error_header()`. The order in which the methods are called is not meaningful.
/// let app = App::new()
///     .wrap(
///         ErrorHandlers::new()
///             .default_handler(add_error_header)
///             .handler(StatusCode::BAD_REQUEST, handle_bad_request)
///     )
///     .service(web::resource("/").route(web::get().to(HttpResponse::InternalServerError)));
/// ```
///
/// You can set default handlers for all client (4xx) or all server (5xx) errors:
///
/// ```
/// # use actix_web::{
/// #     dev::ServiceResponse,
/// #     http::{header, StatusCode},
/// #     middleware::{ErrorHandlerResponse, ErrorHandlers},
/// #     web, App, HttpResponse, Result,
/// # };
/// # fn add_error_header<B>(mut res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
/// #     res.response_mut().headers_mut().insert(
/// #         header::CONTENT_TYPE,
/// #         header::HeaderValue::from_static("Error"),
/// #     );
/// #     Ok(ErrorHandlerResponse::Response(res.map_into_left_body()))
/// # }
/// # fn handle_bad_request<B>(mut res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
/// #     res.response_mut().headers_mut().insert(
/// #         header::CONTENT_TYPE,
/// #         header::HeaderValue::from_static("Bad Request Error"),
/// #     );
/// #     Ok(ErrorHandlerResponse::Response(res.map_into_left_body()))
/// # }
/// // Bad request errors will hit `handle_bad_request()`, other client errors will hit
/// // `add_error_header()`, and server errors will pass through unchanged
/// let app = App::new()
///     .wrap(
///         ErrorHandlers::new()
///             .default_handler_client(add_error_header) // or .default_handler_server
///             .handler(StatusCode::BAD_REQUEST, handle_bad_request)
///     )
///     .service(web::resource("/").route(web::get().to(HttpResponse::InternalServerError)));
/// ```
pub struct ErrorHandlers<B> {
    default_client: DefaultHandler<B>,
    default_server: DefaultHandler<B>,
    handlers: Handlers<B>,
}

type Handlers<B> = Rc<AHashMap<StatusCode, Box<ErrorHandler<B>>>>;

impl<B> Default for ErrorHandlers<B> {
    fn default() -> Self {
        ErrorHandlers {
            default_client: Default::default(),
            default_server: Default::default(),
            handlers: Default::default(),
        }
    }
}

impl<B> ErrorHandlers<B> {
    /// Construct new `ErrorHandlers` instance.
    pub fn new() -> Self {
        ErrorHandlers::default()
    }

    /// Register error handler for specified status code.
    pub fn handler<F>(mut self, status: StatusCode, handler: F) -> Self
    where
        F: Fn(ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> + 'static,
    {
        Rc::get_mut(&mut self.handlers)
            .unwrap()
            .insert(status, Box::new(handler));
        self
    }

    /// Register a default error handler.
    ///
    /// Any request with a status code that hasn't been given a specific other handler (by calling
    /// [`.handler()`][ErrorHandlers::handler]) will fall back on this.
    ///
    /// Note that this will overwrite any default handlers previously set by calling
    /// [`.default_handler_client()`][ErrorHandlers::default_handler_client] or
    /// [`.default_handler_server()`][ErrorHandlers::default_handler_server], but not any set by
    /// calling [`.handler()`][ErrorHandlers::handler].
    pub fn default_handler<F>(self, handler: F) -> Self
    where
        F: Fn(ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> + 'static,
    {
        let handler = Rc::new(handler);
        Self {
            default_server: Some(handler.clone()),
            default_client: Some(handler),
            ..self
        }
    }

    /// Register a handler on which to fall back for client error status codes (400-499).
    pub fn default_handler_client<F>(self, handler: F) -> Self
    where
        F: Fn(ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> + 'static,
    {
        Self {
            default_client: Some(Rc::new(handler)),
            ..self
        }
    }

    /// Register a handler on which to fall back for server error status codes (500-599).
    pub fn default_handler_server<F>(self, handler: F) -> Self
    where
        F: Fn(ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> + 'static,
    {
        Self {
            default_server: Some(Rc::new(handler)),
            ..self
        }
    }

    /// Selects the most appropriate handler for the given status code.
    ///
    /// If the `handlers` map has an entry for that status code, that handler is returned.
    /// Otherwise, fall back on the appropriate default handler.
    fn get_handler<'a>(
        status: &StatusCode,
        default_client: Option<&'a ErrorHandler<B>>,
        default_server: Option<&'a ErrorHandler<B>>,
        handlers: &'a Handlers<B>,
    ) -> Option<&'a ErrorHandler<B>> {
        handlers
            .get(status)
            .map(|h| h.as_ref())
            .or_else(|| status.is_client_error().then_some(default_client).flatten())
            .or_else(|| status.is_server_error().then_some(default_server).flatten())
    }
}

impl<S, B> Transform<S, ServiceRequest> for ErrorHandlers<B>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Transform = ErrorHandlersMiddleware<S, B>;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        let handlers = self.handlers.clone();
        let default_client = self.default_client.clone();
        let default_server = self.default_server.clone();
        Box::pin(async move {
            Ok(ErrorHandlersMiddleware {
                service,
                default_client,
                default_server,
                handlers,
            })
        })
    }
}

#[doc(hidden)]
pub struct ErrorHandlersMiddleware<S, B> {
    service: S,
    default_client: DefaultHandler<B>,
    default_server: DefaultHandler<B>,
    handlers: Handlers<B>,
}

impl<S, B> Service<ServiceRequest> for ErrorHandlersMiddleware<S, B>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = ErrorHandlersFuture<S::Future, B>;

    actix_service::forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let handlers = self.handlers.clone();
        let default_client = self.default_client.clone();
        let default_server = self.default_server.clone();
        let fut = self.service.call(req);
        ErrorHandlersFuture::ServiceFuture {
            fut,
            default_client,
            default_server,
            handlers,
        }
    }
}

pin_project! {
    #[project = ErrorHandlersProj]
    pub enum ErrorHandlersFuture<Fut, B>
    where
        Fut: Future,
    {
        ServiceFuture {
            #[pin]
            fut: Fut,
            default_client: DefaultHandler<B>,
            default_server: DefaultHandler<B>,
            handlers: Handlers<B>,
        },
        ErrorHandlerFuture {
            fut: LocalBoxFuture<'static, Result<ServiceResponse<EitherBody<B>>, Error>>,
        },
    }
}

impl<Fut, B> Future for ErrorHandlersFuture<Fut, B>
where
    Fut: Future<Output = Result<ServiceResponse<B>, Error>>,
{
    type Output = Result<ServiceResponse<EitherBody<B>>, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.as_mut().project() {
            ErrorHandlersProj::ServiceFuture {
                fut,
                default_client,
                default_server,
                handlers,
            } => {
                let res = ready!(fut.poll(cx))?;
                let status = res.status();

                let handler = ErrorHandlers::get_handler(
                    &status,
                    default_client.as_mut().map(|f| Rc::as_ref(f)),
                    default_server.as_mut().map(|f| Rc::as_ref(f)),
                    handlers,
                );
                match handler {
                    Some(handler) => match handler(res)? {
                        ErrorHandlerResponse::Response(res) => Poll::Ready(Ok(res)),
                        ErrorHandlerResponse::Future(fut) => {
                            self.as_mut()
                                .set(ErrorHandlersFuture::ErrorHandlerFuture { fut });

                            self.poll(cx)
                        }
                    },
                    None => Poll::Ready(Ok(res.map_into_left_body())),
                }
            }

            ErrorHandlersProj::ErrorHandlerFuture { fut } => fut.as_mut().poll(cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use actix_service::IntoService;
    use actix_utils::future::ok;
    use bytes::Bytes;
    use futures_util::FutureExt as _;

    use super::*;
    use crate::{
        body,
        http::header::{HeaderValue, CONTENT_TYPE},
        test::{self, TestRequest},
    };

    #[actix_rt::test]
    async fn add_header_error_handler() {
        #[allow(clippy::unnecessary_wraps)]
        fn error_handler<B>(mut res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
            res.response_mut()
                .headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static("0001"));

            Ok(ErrorHandlerResponse::Response(res.map_into_left_body()))
        }

        let srv = test::status_service(StatusCode::INTERNAL_SERVER_ERROR);

        let mw = ErrorHandlers::new()
            .handler(StatusCode::INTERNAL_SERVER_ERROR, error_handler)
            .new_transform(srv.into_service())
            .await
            .unwrap();

        let resp = test::call_service(&mw, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");
    }

    #[actix_rt::test]
    async fn add_header_error_handler_async() {
        #[allow(clippy::unnecessary_wraps)]
        fn error_handler<B: 'static>(
            mut res: ServiceResponse<B>,
        ) -> Result<ErrorHandlerResponse<B>> {
            res.response_mut()
                .headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static("0001"));

            Ok(ErrorHandlerResponse::Future(
                ok(res.map_into_left_body()).boxed_local(),
            ))
        }

        let srv = test::status_service(StatusCode::INTERNAL_SERVER_ERROR);

        let mw = ErrorHandlers::new()
            .handler(StatusCode::INTERNAL_SERVER_ERROR, error_handler)
            .new_transform(srv.into_service())
            .await
            .unwrap();

        let resp = test::call_service(&mw, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");
    }

    #[actix_rt::test]
    async fn changes_body_type() {
        #[allow(clippy::unnecessary_wraps)]
        fn error_handler<B>(res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
            let (req, res) = res.into_parts();
            let res = res.set_body(Bytes::from("sorry, that's no bueno"));

            let res = ServiceResponse::new(req, res)
                .map_into_boxed_body()
                .map_into_right_body();

            Ok(ErrorHandlerResponse::Response(res))
        }

        let srv = test::status_service(StatusCode::INTERNAL_SERVER_ERROR);

        let mw = ErrorHandlers::new()
            .handler(StatusCode::INTERNAL_SERVER_ERROR, error_handler)
            .new_transform(srv.into_service())
            .await
            .unwrap();

        let res = test::call_service(&mw, TestRequest::default().to_srv_request()).await;
        assert_eq!(test::read_body(res).await, "sorry, that's no bueno");
    }

    #[actix_rt::test]
    async fn error_thrown() {
        #[allow(clippy::unnecessary_wraps)]
        fn error_handler<B>(_res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
            Err(crate::error::ErrorInternalServerError(
                "error in error handler",
            ))
        }

        let srv = test::status_service(StatusCode::BAD_REQUEST);

        let mw = ErrorHandlers::new()
            .handler(StatusCode::BAD_REQUEST, error_handler)
            .new_transform(srv.into_service())
            .await
            .unwrap();

        let err = mw
            .call(TestRequest::default().to_srv_request())
            .await
            .unwrap_err();
        let res = err.error_response();

        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            body::to_bytes(res.into_body()).await.unwrap(),
            "error in error handler"
        );
    }

    #[actix_rt::test]
    async fn default_error_handler() {
        #[allow(clippy::unnecessary_wraps)]
        fn error_handler<B>(mut res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
            res.response_mut()
                .headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static("0001"));
            Ok(ErrorHandlerResponse::Response(res.map_into_left_body()))
        }

        let make_mw = |status| async move {
            ErrorHandlers::new()
                .default_handler(error_handler)
                .new_transform(test::status_service(status).into_service())
                .await
                .unwrap()
        };
        let mw_server = make_mw(StatusCode::INTERNAL_SERVER_ERROR).await;
        let mw_client = make_mw(StatusCode::BAD_REQUEST).await;

        let resp = test::call_service(&mw_client, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");

        let resp = test::call_service(&mw_server, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");
    }

    #[actix_rt::test]
    async fn default_handlers_separate_client_server() {
        #[allow(clippy::unnecessary_wraps)]
        fn error_handler_client<B>(mut res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
            res.response_mut()
                .headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static("0001"));
            Ok(ErrorHandlerResponse::Response(res.map_into_left_body()))
        }

        #[allow(clippy::unnecessary_wraps)]
        fn error_handler_server<B>(mut res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
            res.response_mut()
                .headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static("0002"));
            Ok(ErrorHandlerResponse::Response(res.map_into_left_body()))
        }

        let make_mw = |status| async move {
            ErrorHandlers::new()
                .default_handler_server(error_handler_server)
                .default_handler_client(error_handler_client)
                .new_transform(test::status_service(status).into_service())
                .await
                .unwrap()
        };
        let mw_server = make_mw(StatusCode::INTERNAL_SERVER_ERROR).await;
        let mw_client = make_mw(StatusCode::BAD_REQUEST).await;

        let resp = test::call_service(&mw_client, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");

        let resp = test::call_service(&mw_server, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0002");
    }

    #[actix_rt::test]
    async fn default_handlers_specialization() {
        #[allow(clippy::unnecessary_wraps)]
        fn error_handler_client<B>(mut res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
            res.response_mut()
                .headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static("0001"));
            Ok(ErrorHandlerResponse::Response(res.map_into_left_body()))
        }

        #[allow(clippy::unnecessary_wraps)]
        fn error_handler_specific<B>(
            mut res: ServiceResponse<B>,
        ) -> Result<ErrorHandlerResponse<B>> {
            res.response_mut()
                .headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static("0003"));
            Ok(ErrorHandlerResponse::Response(res.map_into_left_body()))
        }

        let make_mw = |status| async move {
            ErrorHandlers::new()
                .default_handler_client(error_handler_client)
                .handler(StatusCode::UNPROCESSABLE_ENTITY, error_handler_specific)
                .new_transform(test::status_service(status).into_service())
                .await
                .unwrap()
        };
        let mw_client = make_mw(StatusCode::BAD_REQUEST).await;
        let mw_specific = make_mw(StatusCode::UNPROCESSABLE_ENTITY).await;

        let resp = test::call_service(&mw_client, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");

        let resp = test::call_service(&mw_specific, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0003");
    }
}
