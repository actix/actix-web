use std::{any::type_name, ops::Deref, sync::Arc};

use actix_http::Extensions;
use actix_utils::future::{err, ok, Ready};
use futures_core::future::LocalBoxFuture;
use serde::{de, Serialize};

use crate::{dev::Payload, error, Error, FromRequest, HttpRequest};

/// Data factory.
pub(crate) trait DataFactory {
    /// Return true if modifications were made to extensions map.
    fn create(&self, extensions: &mut Extensions) -> bool;
}

pub(crate) type FnDataFactory =
    Box<dyn Fn() -> LocalBoxFuture<'static, Result<Box<dyn DataFactory>, ()>>>;

/// Application data wrapper and extractor.
///
/// # Setting Data
/// Data is set using the `app_data` methods on `App`, `Scope`, and `Resource`. If data is wrapped
/// in this `Data` type for those calls, it can be used as an extractor.
///
/// Note that `Data` should be constructed _outside_ the `HttpServer::new` closure if shared,
/// potentially mutable state is desired. `Data` is cheap to clone; internally, it uses an `Arc`.
///
/// See also [`App::app_data`](crate::App::app_data), [`Scope::app_data`](crate::Scope::app_data),
/// and [`Resource::app_data`](crate::Resource::app_data).
///
/// # Extracting `Data`
/// Since the Actix Web router layers application data, the returned object will reference the
/// "closest" instance of the type. For example, if an `App` stores a `u32`, a nested `Scope`
/// also stores a `u32`, and the delegated request handler falls within that `Scope`, then
/// extracting a `web::Data<u32>` for that handler will return the `Scope`'s instance. However,
/// using the same router set up and a request that does not get captured by the `Scope`,
/// `web::<Data<u32>>` would return the `App`'s instance.
///
/// If route data is not set for a handler, using `Data<T>` extractor would cause a `500 Internal
/// Server Error` response.
///
/// See also [`HttpRequest::app_data`]
/// and [`ServiceRequest::app_data`](crate::dev::ServiceRequest::app_data).
///
/// # Unsized Data
/// For types that are unsized, most commonly `dyn T`, `Data` can wrap these types by first
/// constructing an `Arc<dyn T>` and using the `From` implementation to convert it.
///
/// ```
/// # use std::{fmt::Display, sync::Arc};
/// # use actix_web::web::Data;
/// let displayable_arc: Arc<dyn Display> = Arc::new(42usize);
/// let displayable_data: Data<dyn Display> = Data::from(displayable_arc);
/// ```
///
/// # Examples
/// ```
/// use std::sync::Mutex;
/// use actix_web::{App, HttpRequest, HttpResponse, Responder, web::{self, Data}};
///
/// struct MyData {
///     counter: usize,
/// }
///
/// /// Use the `Data<T>` extractor to access data in a handler.
/// async fn index(data: Data<Mutex<MyData>>) -> impl Responder {
///     let mut my_data = data.lock().unwrap();
///     my_data.counter += 1;
///     HttpResponse::Ok()
/// }
///
/// /// Alternatively, use the `HttpRequest::app_data` method to access data in a handler.
/// async fn index_alt(req: HttpRequest) -> impl Responder {
///     let data = req.app_data::<Data<Mutex<MyData>>>().unwrap();
///     let mut my_data = data.lock().unwrap();
///     my_data.counter += 1;
///     HttpResponse::Ok()
/// }
///
/// let data = Data::new(Mutex::new(MyData { counter: 0 }));
///
/// let app = App::new()
///     // Store `MyData` in application storage.
///     .app_data(Data::clone(&data))
///     .route("/index.html", web::get().to(index))
///     .route("/index-alt.html", web::get().to(index_alt));
/// ```
#[doc(alias = "state")]
#[derive(Debug)]
pub struct Data<T: ?Sized>(Arc<T>);

impl<T> Data<T> {
    /// Create new `Data` instance.
    pub fn new(state: T) -> Data<T> {
        Data(Arc::new(state))
    }
}

impl<T: ?Sized> Data<T> {
    /// Returns reference to inner `T`.
    pub fn get_ref(&self) -> &T {
        self.0.as_ref()
    }

    /// Unwraps to the internal `Arc<T>`
    pub fn into_inner(self) -> Arc<T> {
        self.0
    }
}

impl<T: ?Sized> Deref for Data<T> {
    type Target = Arc<T>;

    fn deref(&self) -> &Arc<T> {
        &self.0
    }
}

impl<T: ?Sized> Clone for Data<T> {
    fn clone(&self) -> Data<T> {
        Data(Arc::clone(&self.0))
    }
}

impl<T: ?Sized> From<Arc<T>> for Data<T> {
    fn from(arc: Arc<T>) -> Self {
        Data(arc)
    }
}

impl<T: Default> Default for Data<T> {
    fn default() -> Self {
        Data::new(T::default())
    }
}

impl<T> Serialize for Data<T>
where
    T: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}
impl<'de, T> de::Deserialize<'de> for Data<T>
where
    T: de::Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        Ok(Data::new(T::deserialize(deserializer)?))
    }
}

impl<T: ?Sized + 'static> FromRequest for Data<T> {
    type Error = Error;
    type Future = Ready<Result<Self, Error>>;

    #[inline]
    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        if let Some(st) = req.app_data::<Data<T>>() {
            ok(st.clone())
        } else {
            log::debug!(
                "Failed to extract `Data<{}>` for `{}` handler. For the Data extractor to work \
                correctly, wrap the data with `Data::new()` and pass it to `App::app_data()`. \
                Ensure that types align in both the set and retrieve calls.",
                type_name::<T>(),
                req.match_name().unwrap_or_else(|| req.path())
            );

            err(error::ErrorInternalServerError(
                "Requested application data is not configured correctly. \
                View/enable debug logs for more details.",
            ))
        }
    }
}

impl<T: ?Sized + 'static> DataFactory for Data<T> {
    fn create(&self, extensions: &mut Extensions) -> bool {
        extensions.insert(Data(self.0.clone()));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        dev::Service,
        http::StatusCode,
        test::{init_service, TestRequest},
        web, App, HttpResponse,
    };

    // allow deprecated App::data
    #[allow(deprecated)]
    #[actix_rt::test]
    async fn test_data_extractor() {
        let srv = init_service(
            App::new()
                .data("TEST".to_string())
                .service(web::resource("/").to(|data: web::Data<String>| {
                    assert_eq!(data.to_lowercase(), "test");
                    HttpResponse::Ok()
                })),
        )
        .await;

        let req = TestRequest::default().to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let srv = init_service(
            App::new()
                .data(10u32)
                .service(web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok())),
        )
        .await;
        let req = TestRequest::default().to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let srv = init_service(
            App::new()
                .data(10u32)
                .data(13u32)
                .app_data(12u64)
                .app_data(15u64)
                .default_service(web::to(|n: web::Data<u32>, req: HttpRequest| {
                    // in each case, the latter insertion should be preserved
                    assert_eq!(*req.app_data::<u64>().unwrap(), 15);
                    assert_eq!(*n.into_inner(), 13);
                    HttpResponse::Ok()
                })),
        )
        .await;
        let req = TestRequest::default().to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_app_data_extractor() {
        let srv = init_service(
            App::new()
                .app_data(Data::new(10usize))
                .service(web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok())),
        )
        .await;

        let req = TestRequest::default().to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let srv = init_service(
            App::new()
                .app_data(Data::new(10u32))
                .service(web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok())),
        )
        .await;
        let req = TestRequest::default().to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    // allow deprecated App::data
    #[allow(deprecated)]
    #[actix_rt::test]
    async fn test_route_data_extractor() {
        let srv = init_service(
            App::new().service(
                web::resource("/")
                    .data(10usize)
                    .route(web::get().to(|_data: web::Data<usize>| HttpResponse::Ok())),
            ),
        )
        .await;

        let req = TestRequest::default().to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // different type
        let srv = init_service(
            App::new().service(
                web::resource("/")
                    .data(10u32)
                    .route(web::get().to(|_: web::Data<usize>| HttpResponse::Ok())),
            ),
        )
        .await;
        let req = TestRequest::default().to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    // allow deprecated App::data
    #[allow(deprecated)]
    #[actix_rt::test]
    async fn test_override_data() {
        let srv = init_service(
            App::new()
                .data(1usize)
                .service(web::resource("/").data(10usize).route(web::get().to(
                    |data: web::Data<usize>| {
                        assert_eq!(**data, 10);
                        HttpResponse::Ok()
                    },
                ))),
        )
        .await;

        let req = TestRequest::default().to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_data_from_arc() {
        let data_new = Data::new(String::from("test-123"));
        let data_from_arc = Data::from(Arc::new(String::from("test-123")));
        assert_eq!(data_new.0, data_from_arc.0);
    }

    #[actix_rt::test]
    async fn test_data_from_dyn_arc() {
        trait TestTrait {
            fn get_num(&self) -> i32;
        }
        struct A {}
        impl TestTrait for A {
            fn get_num(&self) -> i32 {
                42
            }
        }
        // This works when Sized is required
        let dyn_arc_box: Arc<Box<dyn TestTrait>> = Arc::new(Box::new(A {}));
        let data_arc_box = Data::from(dyn_arc_box);
        // This works when Data Sized Bound is removed
        let dyn_arc: Arc<dyn TestTrait> = Arc::new(A {});
        let data_arc = Data::from(dyn_arc);
        assert_eq!(data_arc_box.get_num(), data_arc.get_num())
    }

    #[actix_rt::test]
    async fn test_dyn_data_into_arc() {
        trait TestTrait {
            fn get_num(&self) -> i32;
        }
        struct A {}
        impl TestTrait for A {
            fn get_num(&self) -> i32 {
                42
            }
        }
        let dyn_arc: Arc<dyn TestTrait> = Arc::new(A {});
        let data_arc = Data::from(dyn_arc);
        let arc_from_data = data_arc.clone().into_inner();
        assert_eq!(data_arc.get_num(), arc_from_data.get_num())
    }

    #[actix_rt::test]
    async fn test_get_ref_from_dyn_data() {
        trait TestTrait {
            fn get_num(&self) -> i32;
        }
        struct A {}
        impl TestTrait for A {
            fn get_num(&self) -> i32 {
                42
            }
        }
        let dyn_arc: Arc<dyn TestTrait> = Arc::new(A {});
        let data_arc = Data::from(dyn_arc);
        let ref_data = data_arc.get_ref();
        assert_eq!(data_arc.get_num(), ref_data.get_num())
    }
}
