//! Request extractors

use actix_http::error::Error;
use futures::future::ok;
use futures::{future, Async, Future, IntoFuture, Poll};

use crate::service::ServiceFromRequest;

/// Trait implemented by types that can be extracted from request.
///
/// Types that implement this trait can be used with `Route` handlers.
pub trait FromRequest<P>: Sized {
    /// The associated error which can be returned.
    type Error: Into<Error>;

    /// Future that resolves to a Self
    type Future: IntoFuture<Item = Self, Error = Self::Error>;

    /// Convert request to a Self
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future;
}

/// Optionally extract a field from the request
///
/// If the FromRequest for T fails, return None rather than returning an error response
///
/// ## Example
///
/// ```rust
/// # #[macro_use] extern crate serde_derive;
/// use actix_web::{web, dev, App, Error, FromRequest};
/// use actix_web::error::ErrorBadRequest;
/// use rand;
///
/// #[derive(Debug, Deserialize)]
/// struct Thing {
///     name: String
/// }
///
/// impl<P> FromRequest<P> for Thing {
///     type Error = Error;
///     type Future = Result<Self, Self::Error>;
///
///     fn from_request(req: &mut dev::ServiceFromRequest<P>) -> Self::Future {
///         if rand::random() {
///             Ok(Thing { name: "thingy".into() })
///         } else {
///             Err(ErrorBadRequest("no luck"))
///         }
///
///     }
/// }
///
/// /// extract `Thing` from request
/// fn index(supplied_thing: Option<Thing>) -> String {
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
impl<T: 'static, P> FromRequest<P> for Option<T>
where
    T: FromRequest<P>,
    T::Future: 'static,
{
    type Error = Error;
    type Future = Box<Future<Item = Option<T>, Error = Error>>;

    #[inline]
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        Box::new(T::from_request(req).into_future().then(|r| match r {
            Ok(v) => future::ok(Some(v)),
            Err(e) => {
                log::debug!("Error for Option<T> extractor: {}", e.into());
                future::ok(None)
            }
        }))
    }
}

/// Optionally extract a field from the request or extract the Error if unsuccessful
///
/// If the `FromRequest` for T fails, inject Err into handler rather than returning an error response
///
/// ## Example
///
/// ```rust
/// # #[macro_use] extern crate serde_derive;
/// use actix_web::{web, dev, App, Result, Error, FromRequest};
/// use actix_web::error::ErrorBadRequest;
/// use rand;
///
/// #[derive(Debug, Deserialize)]
/// struct Thing {
///     name: String
/// }
///
/// impl<P> FromRequest<P> for Thing {
///     type Error = Error;
///     type Future = Result<Thing, Error>;
///
///     fn from_request(req: &mut dev::ServiceFromRequest<P>) -> Self::Future {
///         if rand::random() {
///             Ok(Thing { name: "thingy".into() })
///         } else {
///             Err(ErrorBadRequest("no luck"))
///         }
///     }
/// }
///
/// /// extract `Thing` from request
/// fn index(supplied_thing: Result<Thing>) -> String {
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
impl<T: 'static, P> FromRequest<P> for Result<T, T::Error>
where
    T: FromRequest<P>,
    T::Future: 'static,
    T::Error: 'static,
{
    type Error = Error;
    type Future = Box<Future<Item = Result<T, T::Error>, Error = Error>>;

    #[inline]
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        Box::new(T::from_request(req).into_future().then(|res| match res {
            Ok(v) => ok(Ok(v)),
            Err(e) => ok(Err(e)),
        }))
    }
}

#[doc(hidden)]
impl<P> FromRequest<P> for () {
    type Error = Error;
    type Future = Result<(), Error>;

    fn from_request(_req: &mut ServiceFromRequest<P>) -> Self::Future {
        Ok(())
    }
}

macro_rules! tuple_from_req ({$fut_type:ident, $(($n:tt, $T:ident)),+} => {

    /// FromRequest implementation for tuple
    #[doc(hidden)]
    impl<P, $($T: FromRequest<P> + 'static),+> FromRequest<P> for ($($T,)+)
    {
        type Error = Error;
        type Future = $fut_type<P, $($T),+>;

        fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
            $fut_type {
                items: <($(Option<$T>,)+)>::default(),
                futs: ($($T::from_request(req).into_future(),)+),
            }
        }
    }

    #[doc(hidden)]
    pub struct $fut_type<P, $($T: FromRequest<P>),+> {
        items: ($(Option<$T>,)+),
        futs: ($(<$T::Future as futures::IntoFuture>::Future,)+),
    }

    impl<P, $($T: FromRequest<P>),+> Future for $fut_type<P, $($T),+>
    {
        type Item = ($($T,)+);
        type Error = Error;

        fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
            let mut ready = true;

            $(
                if self.items.$n.is_none() {
                    match self.futs.$n.poll() {
                        Ok(Async::Ready(item)) => {
                            self.items.$n = Some(item);
                        }
                        Ok(Async::NotReady) => ready = false,
                        Err(e) => return Err(e.into()),
                    }
                }
            )+

                if ready {
                    Ok(Async::Ready(
                        ($(self.items.$n.take().unwrap(),)+)
                    ))
                } else {
                    Ok(Async::NotReady)
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
    use actix_router::ResourceDef;
    use bytes::Bytes;
    use serde_derive::Deserialize;

    use super::*;
    use crate::test::{block_on, TestRequest};
    use crate::types::{Form, FormConfig, Path, Query};

    #[derive(Deserialize, Debug, PartialEq)]
    struct Info {
        hello: String,
    }

    #[test]
    fn test_option() {
        let mut req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .route_data(FormConfig::default().limit(4096))
        .to_from();

        let r = block_on(Option::<Form<Info>>::from_request(&mut req)).unwrap();
        assert_eq!(r, None);

        let mut req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(header::CONTENT_LENGTH, "9")
        .set_payload(Bytes::from_static(b"hello=world"))
        .to_from();

        let r = block_on(Option::<Form<Info>>::from_request(&mut req)).unwrap();
        assert_eq!(
            r,
            Some(Form(Info {
                hello: "world".into()
            }))
        );

        let mut req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(header::CONTENT_LENGTH, "9")
        .set_payload(Bytes::from_static(b"bye=world"))
        .to_from();

        let r = block_on(Option::<Form<Info>>::from_request(&mut req)).unwrap();
        assert_eq!(r, None);
    }

    #[test]
    fn test_result() {
        let mut req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(header::CONTENT_LENGTH, "11")
        .set_payload(Bytes::from_static(b"hello=world"))
        .to_from();

        let r = block_on(Result::<Form<Info>, Error>::from_request(&mut req))
            .unwrap()
            .unwrap();
        assert_eq!(
            r,
            Form(Info {
                hello: "world".into()
            })
        );

        let mut req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(header::CONTENT_LENGTH, "9")
        .set_payload(Bytes::from_static(b"bye=world"))
        .to_from();

        let r = block_on(Result::<Form<Info>, Error>::from_request(&mut req)).unwrap();
        assert!(r.is_err());
    }

    #[derive(Deserialize)]
    struct MyStruct {
        key: String,
        value: String,
    }

    #[derive(Deserialize)]
    struct Id {
        id: String,
    }

    #[derive(Deserialize)]
    struct Test2 {
        key: String,
        value: u32,
    }

    #[test]
    fn test_request_extract() {
        let mut req = TestRequest::with_uri("/name/user1/?id=test").to_from();

        let resource = ResourceDef::new("/{key}/{value}/");
        resource.match_path(req.match_info_mut());

        let s = Path::<MyStruct>::from_request(&mut req).unwrap();
        assert_eq!(s.key, "name");
        assert_eq!(s.value, "user1");

        let s = Path::<(String, String)>::from_request(&mut req).unwrap();
        assert_eq!(s.0, "name");
        assert_eq!(s.1, "user1");

        let s = Query::<Id>::from_request(&mut req).unwrap();
        assert_eq!(s.id, "test");

        let mut req = TestRequest::with_uri("/name/32/").to_from();
        let resource = ResourceDef::new("/{key}/{value}/");
        resource.match_path(req.match_info_mut());

        let s = Path::<Test2>::from_request(&mut req).unwrap();
        assert_eq!(s.as_ref().key, "name");
        assert_eq!(s.value, 32);

        let s = Path::<(String, u8)>::from_request(&mut req).unwrap();
        assert_eq!(s.0, "name");
        assert_eq!(s.1, 32);

        let res = Path::<Vec<String>>::from_request(&mut req).unwrap();
        assert_eq!(res[0], "name".to_owned());
        assert_eq!(res[1], "32".to_owned());
    }

}
