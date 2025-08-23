//! Request extractors

use std::{
    convert::Infallible,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use actix_http::{Method, Uri};
use actix_utils::future::{ok, Ready};
use futures_core::ready;
use pin_project_lite::pin_project;

use crate::{dev::Payload, Error, HttpRequest};

/// A type that implements [`FromRequest`] is called an **extractor** and can extract data from
/// the request. Some types that implement this trait are: [`Json`], [`Header`], and [`Path`].
///
/// Check out [`ServiceRequest::extract`](crate::dev::ServiceRequest::extract) if you want to
/// leverage extractors when implementing middlewares.
///
/// # Configuration
/// An extractor can be customized by injecting the corresponding configuration with one of:
/// - [`App::app_data()`][crate::App::app_data]
/// - [`Scope::app_data()`][crate::Scope::app_data]
/// - [`Resource::app_data()`][crate::Resource::app_data]
///
/// Here are some built-in extractors and their corresponding configuration.
/// Please refer to the respective documentation for details.
///
/// | Extractor   | Configuration     |
/// |-------------|-------------------|
/// | [`Header`]  | _None_            |
/// | [`Path`]    | [`PathConfig`]    |
/// | [`Json`]    | [`JsonConfig`]    |
/// | [`Form`]    | [`FormConfig`]    |
/// | [`Query`]   | [`QueryConfig`]   |
/// | [`Bytes`]   | [`PayloadConfig`] |
/// | [`String`]  | [`PayloadConfig`] |
/// | [`Payload`] | [`PayloadConfig`] |
///
/// # Implementing An Extractor
/// To reduce duplicate code in handlers where extracting certain parts of a request has a common
/// structure, you can implement `FromRequest` for your own types.
///
/// Note that the request payload can only be consumed by one extractor.
///
/// [`Header`]: crate::web::Header
/// [`Json`]: crate::web::Json
/// [`JsonConfig`]: crate::web::JsonConfig
/// [`Form`]: crate::web::Form
/// [`FormConfig`]: crate::web::FormConfig
/// [`Path`]: crate::web::Path
/// [`PathConfig`]: crate::web::PathConfig
/// [`Query`]: crate::web::Query
/// [`QueryConfig`]: crate::web::QueryConfig
/// [`Payload`]: crate::web::Payload
/// [`PayloadConfig`]: crate::web::PayloadConfig
/// [`String`]: FromRequest#impl-FromRequest-for-String
/// [`Bytes`]: crate::web::Bytes#impl-FromRequest
/// [`Either`]: crate::web::Either
#[doc(alias = "extract", alias = "extractor")]
pub trait FromRequest: Sized {
    /// The associated error which can be returned.
    type Error: Into<Error>;

    /// Future that resolves to a `Self`.
    ///
    /// To use an async function or block, the futures must be boxed. The following snippet will be
    /// common when creating async/await extractors (that do not consume the body).
    ///
    /// ```ignore
    /// type Future = Pin<Box<dyn Future<Output = Result<Self, Self::Error>>>>;
    /// // or
    /// type Future = futures_util::future::LocalBoxFuture<'static, Result<Self, Self::Error>>;
    ///
    /// fn from_request(req: HttpRequest, ...) -> Self::Future {
    ///     let req = req.clone();
    ///     Box::pin(async move {
    ///         ...
    ///     })
    /// }
    /// ```
    type Future: Future<Output = Result<Self, Self::Error>>;

    /// Create a `Self` from request parts asynchronously.
    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future;

    /// Create a `Self` from request head asynchronously.
    ///
    /// This method is short for `T::from_request(req, &mut Payload::None)`.
    fn extract(req: &HttpRequest) -> Self::Future {
        Self::from_request(req, &mut Payload::None)
    }
}

/// Optionally extract from the request.
///
/// If the inner `T::from_request` returns an error, handler will receive `None` instead.
///
/// # Examples
/// ```
/// use actix_web::{web, dev, App, Error, HttpRequest, FromRequest};
/// use actix_web::error::ErrorBadRequest;
/// use futures_util::future::{ok, err, Ready};
/// use serde::Deserialize;
/// use rand;
///
/// #[derive(Debug, Deserialize)]
/// struct Thing {
///     name: String
/// }
///
/// impl FromRequest for Thing {
///     type Error = Error;
///     type Future = Ready<Result<Self, Self::Error>>;
///
///     fn from_request(req: &HttpRequest, payload: &mut dev::Payload) -> Self::Future {
///         if rand::random() {
///             ok(Thing { name: "thingy".into() })
///         } else {
///             err(ErrorBadRequest("no luck"))
///         }
///
///     }
/// }
///
/// /// extract `Thing` from request
/// async fn index(supplied_thing: Option<Thing>) -> String {
///     match supplied_thing {
///         // Puns not intended
///         Some(thing) => format!("Got something: {:?}", thing),
///         None => format!("No thing!")
///     }
/// }
///
/// let app = App::new().service(
///     web::resource("/users/:first").route(
///         web::post().to(index))
/// );
/// ```
impl<T> FromRequest for Option<T>
where
    T: FromRequest,
{
    type Error = Infallible;
    type Future = FromRequestOptFuture<T::Future>;

    #[inline]
    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        FromRequestOptFuture {
            fut: T::from_request(req, payload),
        }
    }
}

pin_project! {
    pub struct FromRequestOptFuture<Fut> {
        #[pin]
        fut: Fut,
    }
}

impl<Fut, T, E> Future for FromRequestOptFuture<Fut>
where
    Fut: Future<Output = Result<T, E>>,
    E: Into<Error>,
{
    type Output = Result<Option<T>, Infallible>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let res = ready!(this.fut.poll(cx));
        match res {
            Ok(t) => Poll::Ready(Ok(Some(t))),
            Err(err) => {
                log::debug!("Error for Option<T> extractor: {}", err.into());
                Poll::Ready(Ok(None))
            }
        }
    }
}

/// Extract from the request, passing error type through to handler.
///
/// If the inner `T::from_request` returns an error, allow handler to receive the error rather than
/// immediately returning an error response.
///
/// # Examples
/// ```
/// use actix_web::{web, dev, App, Result, Error, HttpRequest, FromRequest};
/// use actix_web::error::ErrorBadRequest;
/// use futures_util::future::{ok, err, Ready};
/// use serde::Deserialize;
/// use rand;
///
/// #[derive(Debug, Deserialize)]
/// struct Thing {
///     name: String
/// }
///
/// impl FromRequest for Thing {
///     type Error = Error;
///     type Future = Ready<Result<Thing, Error>>;
///
///     fn from_request(req: &HttpRequest, payload: &mut dev::Payload) -> Self::Future {
///         if rand::random() {
///             ok(Thing { name: "thingy".into() })
///         } else {
///             err(ErrorBadRequest("no luck"))
///         }
///     }
/// }
///
/// /// extract `Thing` from request
/// async fn index(supplied_thing: Result<Thing>) -> String {
///     match supplied_thing {
///         Ok(thing) => format!("Got thing: {thing:?}"),
///         Err(err) => format!("Error extracting thing: {err}"),
///     }
/// }
///
/// let app = App::new().service(
///     web::resource("/users/:first").route(web::post().to(index))
/// );
/// ```
impl<T, E> FromRequest for Result<T, E>
where
    T: FromRequest,
    T::Error: Into<E>,
{
    type Error = Infallible;
    type Future = FromRequestResFuture<T::Future, E>;

    #[inline]
    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        FromRequestResFuture {
            fut: T::from_request(req, payload),
            _phantom: PhantomData,
        }
    }
}

pin_project! {
    pub struct FromRequestResFuture<Fut, E> {
        #[pin]
        fut: Fut,
        _phantom: PhantomData<E>,
    }
}

impl<Fut, T, Ei, E> Future for FromRequestResFuture<Fut, E>
where
    Fut: Future<Output = Result<T, Ei>>,
    Ei: Into<E>,
{
    type Output = Result<Result<T, E>, Infallible>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let res = ready!(this.fut.poll(cx));
        Poll::Ready(Ok(res.map_err(Into::into)))
    }
}

/// Extract the request's URI.
///
/// # Examples
/// ```
/// use actix_web::{http::Uri, web, App, Responder};
///
/// async fn handler(uri: Uri) -> impl Responder {
///     format!("Requested path: {}", uri.path())
/// }
///
/// let app = App::new().default_service(web::to(handler));
/// ```
impl FromRequest for Uri {
    type Error = Infallible;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        ok(req.uri().clone())
    }
}

/// Extract the request's method.
///
/// # Examples
/// ```
/// use actix_web::{http::Method, web, App, Responder};
///
/// async fn handler(method: Method) -> impl Responder {
///     format!("Request method: {}", method)
/// }
///
/// let app = App::new().default_service(web::to(handler));
/// ```
impl FromRequest for Method {
    type Error = Infallible;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        ok(req.method().clone())
    }
}

#[doc(hidden)]
#[allow(non_snake_case)]
mod tuple_from_req {
    use super::*;

    macro_rules! tuple_from_req {
        ($fut: ident; $($T: ident),*) => {
            /// FromRequest implementation for tuple
            #[allow(unused_parens)]
            impl<$($T: FromRequest + 'static),+> FromRequest for ($($T,)+)
            {
                type Error = Error;
                type Future = $fut<$($T),+>;

                fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
                    $fut {
                        $(
                            $T: ExtractFuture::Future {
                                fut: $T::from_request(req, payload)
                            },
                        )+
                    }
                }
            }

            pin_project! {
                pub struct $fut<$($T: FromRequest),+> {
                    $(
                        #[pin]
                        $T: ExtractFuture<$T::Future, $T>,
                    )+
                }
            }

            impl<$($T: FromRequest),+> Future for $fut<$($T),+>
            {
                type Output = Result<($($T,)+), Error>;

                fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                    let mut this = self.project();

                    let mut ready = true;
                    $(
                        match this.$T.as_mut().project() {
                            ExtractProj::Future { fut } => match fut.poll(cx) {
                                Poll::Ready(Ok(output)) => {
                                    let _ = this.$T.as_mut().project_replace(ExtractFuture::Done { output });
                                },
                                Poll::Ready(Err(err)) => return Poll::Ready(Err(err.into())),
                                Poll::Pending => ready = false,
                            },
                            ExtractProj::Done { .. } => {},
                            ExtractProj::Empty => unreachable!("FromRequest polled after finished"),
                        }
                    )+

                    if ready {
                        Poll::Ready(Ok(
                            ($(
                                match this.$T.project_replace(ExtractFuture::Empty) {
                                    ExtractReplaceProj::Done { output } => output,
                                    _ => unreachable!("FromRequest polled after finished"),
                                },
                            )+)
                        ))
                    } else {
                        Poll::Pending
                    }
                }
            }
        };
    }

    pin_project! {
        #[project = ExtractProj]
        #[project_replace = ExtractReplaceProj]
        enum ExtractFuture<Fut, Res> {
            Future {
                #[pin]
                fut: Fut
            },
            Done {
                output: Res,
            },
            Empty
        }
    }

    impl FromRequest for () {
        type Error = Infallible;
        type Future = Ready<Result<Self, Self::Error>>;

        fn from_request(_: &HttpRequest, _: &mut Payload) -> Self::Future {
            ok(())
        }
    }

    tuple_from_req! { TupleFromRequest1; A }
    tuple_from_req! { TupleFromRequest2; A, B }
    tuple_from_req! { TupleFromRequest3; A, B, C }
    tuple_from_req! { TupleFromRequest4; A, B, C, D }
    tuple_from_req! { TupleFromRequest5; A, B, C, D, E }
    tuple_from_req! { TupleFromRequest6; A, B, C, D, E, F }
    tuple_from_req! { TupleFromRequest7; A, B, C, D, E, F, G }
    tuple_from_req! { TupleFromRequest8; A, B, C, D, E, F, G, H }
    tuple_from_req! { TupleFromRequest9; A, B, C, D, E, F, G, H, I }
    tuple_from_req! { TupleFromRequest10; A, B, C, D, E, F, G, H, I, J }
    tuple_from_req! { TupleFromRequest11; A, B, C, D, E, F, G, H, I, J, K }
    tuple_from_req! { TupleFromRequest12; A, B, C, D, E, F, G, H, I, J, K, L }
    tuple_from_req! { TupleFromRequest13; A, B, C, D, E, F, G, H, I, J, K, L, M }
    tuple_from_req! { TupleFromRequest14; A, B, C, D, E, F, G, H, I, J, K, L, M, N }
    tuple_from_req! { TupleFromRequest15; A, B, C, D, E, F, G, H, I, J, K, L, M, N, O }
    tuple_from_req! { TupleFromRequest16; A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P }
}

#[cfg(test)]
mod tests {
    use actix_http::header;
    use bytes::Bytes;
    use serde::Deserialize;

    use super::*;
    use crate::{
        test::TestRequest,
        types::{Form, FormConfig},
    };

    #[derive(Deserialize, Debug, PartialEq)]
    struct Info {
        hello: String,
    }

    #[actix_rt::test]
    async fn test_option() {
        let (req, mut pl) = TestRequest::default()
            .insert_header((header::CONTENT_TYPE, "application/x-www-form-urlencoded"))
            .data(FormConfig::default().limit(4096))
            .to_http_parts();

        let r = Option::<Form<Info>>::from_request(&req, &mut pl)
            .await
            .unwrap();
        assert_eq!(r, None);

        let (req, mut pl) = TestRequest::default()
            .insert_header((header::CONTENT_TYPE, "application/x-www-form-urlencoded"))
            .insert_header((header::CONTENT_LENGTH, "9"))
            .set_payload(Bytes::from_static(b"hello=world"))
            .to_http_parts();

        let r = Option::<Form<Info>>::from_request(&req, &mut pl)
            .await
            .unwrap();
        assert_eq!(
            r,
            Some(Form(Info {
                hello: "world".into()
            }))
        );

        let (req, mut pl) = TestRequest::default()
            .insert_header((header::CONTENT_TYPE, "application/x-www-form-urlencoded"))
            .insert_header((header::CONTENT_LENGTH, "9"))
            .set_payload(Bytes::from_static(b"bye=world"))
            .to_http_parts();

        let r = Option::<Form<Info>>::from_request(&req, &mut pl)
            .await
            .unwrap();
        assert_eq!(r, None);
    }

    #[actix_rt::test]
    async fn test_result() {
        let (req, mut pl) = TestRequest::default()
            .insert_header((header::CONTENT_TYPE, "application/x-www-form-urlencoded"))
            .insert_header((header::CONTENT_LENGTH, "11"))
            .set_payload(Bytes::from_static(b"hello=world"))
            .to_http_parts();

        let r = Result::<Form<Info>, Error>::from_request(&req, &mut pl)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            r,
            Form(Info {
                hello: "world".into()
            })
        );

        let (req, mut pl) = TestRequest::default()
            .insert_header((header::CONTENT_TYPE, "application/x-www-form-urlencoded"))
            .insert_header((header::CONTENT_LENGTH, 9))
            .set_payload(Bytes::from_static(b"bye=world"))
            .to_http_parts();

        struct MyError;
        impl From<Error> for MyError {
            fn from(_: Error) -> Self {
                Self
            }
        }

        let r = Result::<Form<Info>, MyError>::from_request(&req, &mut pl)
            .await
            .unwrap();
        assert!(r.is_err());
    }

    #[actix_rt::test]
    async fn test_uri() {
        let req = TestRequest::default().uri("/foo/bar").to_http_request();
        let uri = Uri::extract(&req).await.unwrap();
        assert_eq!(uri.path(), "/foo/bar");
    }

    #[actix_rt::test]
    async fn test_method() {
        let req = TestRequest::default().method(Method::GET).to_http_request();
        let method = Method::extract(&req).await.unwrap();
        assert_eq!(method, Method::GET);
    }

    #[actix_rt::test]
    async fn test_concurrent() {
        let (req, mut pl) = TestRequest::default()
            .uri("/foo/bar")
            .method(Method::GET)
            .insert_header((header::CONTENT_TYPE, "application/x-www-form-urlencoded"))
            .insert_header((header::CONTENT_LENGTH, "11"))
            .set_payload(Bytes::from_static(b"hello=world"))
            .to_http_parts();
        let (method, uri, form) = <(Method, Uri, Form<Info>)>::from_request(&req, &mut pl)
            .await
            .unwrap();
        assert_eq!(method, Method::GET);
        assert_eq!(uri.path(), "/foo/bar");
        assert_eq!(
            form,
            Form(Info {
                hello: "world".into()
            })
        );
    }
}
