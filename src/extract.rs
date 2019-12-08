//! Request extractors
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use actix_http::error::Error;
use futures::future::{ok, FutureExt, LocalBoxFuture, Ready};

use crate::dev::Payload;
use crate::request::HttpRequest;

/// Trait implemented by types that can be extracted from request.
///
/// Types that implement this trait can be used with `Route` handlers.
pub trait FromRequest: Sized {
    /// The associated error which can be returned.
    type Error: Into<Error>;

    /// Future that resolves to a Self
    type Future: Future<Output = Result<Self, Self::Error>>;

    /// Configuration for this extractor
    type Config: Default + 'static;

    /// Convert request to a Self
    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future;

    /// Convert request to a Self
    ///
    /// This method uses `Payload::None` as payload stream.
    fn extract(req: &HttpRequest) -> Self::Future {
        Self::from_request(req, &mut Payload::None)
    }

    /// Create and configure config instance.
    fn configure<F>(f: F) -> Self::Config
    where
        F: FnOnce(Self::Config) -> Self::Config,
    {
        f(Self::Config::default())
    }
}

/// Optionally extract a field from the request
///
/// If the FromRequest for T fails, return None rather than returning an error response
///
/// ## Example
///
/// ```rust
/// use actix_web::{web, dev, App, Error, HttpRequest, FromRequest};
/// use actix_web::error::ErrorBadRequest;
/// use futures::future::{ok, err, Ready};
/// use serde_derive::Deserialize;
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
///     type Config = ();
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
/// fn main() {
///     let app = App::new().service(
///         web::resource("/users/:first").route(
///             web::post().to(index))
///     );
/// }
/// ```
impl<T: 'static> FromRequest for Option<T>
where
    T: FromRequest,
    T::Future: 'static,
{
    type Config = T::Config;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Option<T>, Error>>;

    #[inline]
    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        T::from_request(req, payload)
            .then(|r| match r {
                Ok(v) => ok(Some(v)),
                Err(e) => {
                    log::debug!("Error for Option<T> extractor: {}", e.into());
                    ok(None)
                }
            })
            .boxed_local()
    }
}

/// Optionally extract a field from the request or extract the Error if unsuccessful
///
/// If the `FromRequest` for T fails, inject Err into handler rather than returning an error response
///
/// ## Example
///
/// ```rust
/// use actix_web::{web, dev, App, Result, Error, HttpRequest, FromRequest};
/// use actix_web::error::ErrorBadRequest;
/// use futures::future::{ok, err, Ready};
/// use serde_derive::Deserialize;
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
///     type Config = ();
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
///         Ok(thing) => format!("Got thing: {:?}", thing),
///         Err(e) => format!("Error extracting thing: {}", e)
///     }
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/users/:first").route(web::post().to(index))
///     );
/// }
/// ```
impl<T> FromRequest for Result<T, T::Error>
where
    T: FromRequest + 'static,
    T::Error: 'static,
    T::Future: 'static,
{
    type Config = T::Config;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Result<T, T::Error>, Error>>;

    #[inline]
    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        T::from_request(req, payload)
            .then(|res| match res {
                Ok(v) => ok(Ok(v)),
                Err(e) => ok(Err(e)),
            })
            .boxed_local()
    }
}

#[doc(hidden)]
impl FromRequest for () {
    type Config = ();
    type Error = Error;
    type Future = Ready<Result<(), Error>>;

    fn from_request(_: &HttpRequest, _: &mut Payload) -> Self::Future {
        ok(())
    }
}

macro_rules! tuple_from_req ({$fut_type:ident, $(($n:tt, $T:ident)),+} => {

    /// FromRequest implementation for tuple
    #[doc(hidden)]
    #[allow(unused_parens)]
    impl<$($T: FromRequest + 'static),+> FromRequest for ($($T,)+)
    {
        type Error = Error;
        type Future = $fut_type<$($T),+>;
        type Config = ($($T::Config),+);

        fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
            $fut_type {
                items: <($(Option<$T>,)+)>::default(),
                futs: ($($T::from_request(req, payload),)+),
            }
        }
    }

    #[doc(hidden)]
    #[pin_project::pin_project]
    pub struct $fut_type<$($T: FromRequest),+> {
        items: ($(Option<$T>,)+),
        futs: ($($T::Future,)+),
    }

    impl<$($T: FromRequest),+> Future for $fut_type<$($T),+>
    {
        type Output = Result<($($T,)+), Error>;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let this = self.project();

            let mut ready = true;
            $(
                if this.items.$n.is_none() {
                    match unsafe { Pin::new_unchecked(&mut this.futs.$n) }.poll(cx) {
                        Poll::Ready(Ok(item)) => {
                            this.items.$n = Some(item);
                        }
                        Poll::Pending => ready = false,
                        Poll::Ready(Err(e)) => return Poll::Ready(Err(e.into())),
                    }
                }
            )+

                if ready {
                    Poll::Ready(Ok(
                        ($(this.items.$n.take().unwrap(),)+)
                    ))
                } else {
                    Poll::Pending
                }
        }
    }
});

#[rustfmt::skip]
mod m {
    use super::*;

tuple_from_req!(TupleFromRequest1, (0, A));
tuple_from_req!(TupleFromRequest2, (0, A), (1, B));
tuple_from_req!(TupleFromRequest3, (0, A), (1, B), (2, C));
tuple_from_req!(TupleFromRequest4, (0, A), (1, B), (2, C), (3, D));
tuple_from_req!(TupleFromRequest5, (0, A), (1, B), (2, C), (3, D), (4, E));
tuple_from_req!(TupleFromRequest6, (0, A), (1, B), (2, C), (3, D), (4, E), (5, F));
tuple_from_req!(TupleFromRequest7, (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G));
tuple_from_req!(TupleFromRequest8, (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H));
tuple_from_req!(TupleFromRequest9, (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I));
tuple_from_req!(TupleFromRequest10, (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I), (9, J));
}

#[cfg(test)]
mod tests {
    use actix_http::http::header;
    use bytes::Bytes;
    use serde_derive::Deserialize;

    use super::*;
    use crate::test::TestRequest;
    use crate::types::{Form, FormConfig};

    #[derive(Deserialize, Debug, PartialEq)]
    struct Info {
        hello: String,
    }

    #[actix_rt::test]
    async fn test_option() {
        let (req, mut pl) = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .data(FormConfig::default().limit(4096))
        .to_http_parts();

        let r = Option::<Form<Info>>::from_request(&req, &mut pl)
            .await
            .unwrap();
        assert_eq!(r, None);

        let (req, mut pl) = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(header::CONTENT_LENGTH, "9")
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

        let (req, mut pl) = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(header::CONTENT_LENGTH, "9")
        .set_payload(Bytes::from_static(b"bye=world"))
        .to_http_parts();

        let r = Option::<Form<Info>>::from_request(&req, &mut pl)
            .await
            .unwrap();
        assert_eq!(r, None);
    }

    #[actix_rt::test]
    async fn test_result() {
        let (req, mut pl) = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(header::CONTENT_LENGTH, "11")
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

        let (req, mut pl) = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(header::CONTENT_LENGTH, "9")
        .set_payload(Bytes::from_static(b"bye=world"))
        .to_http_parts();

        let r = Result::<Form<Info>, Error>::from_request(&req, &mut pl)
            .await
            .unwrap();
        assert!(r.is_err());
    }
}
