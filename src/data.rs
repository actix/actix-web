use std::any::type_name;
use std::ops::Deref;
use std::sync::Arc;

use actix_http::error::{Error, ErrorInternalServerError};
use actix_http::Extensions;
use futures_util::future::{err, ok, LocalBoxFuture, Ready};

use crate::dev::Payload;
use crate::extract::FromRequest;
use crate::request::HttpRequest;

/// Application data factory
pub(crate) trait DataFactory {
    fn create(&self, extensions: &mut Extensions) -> bool;
}

pub(crate) type FnDataFactory =
    Box<dyn Fn() -> LocalBoxFuture<'static, Result<Box<dyn DataFactory>, ()>>>;

/// Application data.
///
/// Application level data is a piece of arbitrary data attached to the app, scope, or resource.
/// Application data is available to all routes and can be added during the application
/// configuration process via `App::data()`.
///
/// Application data can be accessed by using `Data<T>` extractor where `T` is data type.
///
/// **Note**: http server accepts an application factory rather than an application instance. HTTP
/// server constructs an application instance for each thread, thus application data must be
/// constructed multiple times. If you want to share data between different threads, a shareable
/// object should be used, e.g. `Send + Sync`. Application data does not need to be `Send`
/// or `Sync`. Internally `Data` uses `Arc`.
///
/// If route data is not set for a handler, using `Data<T>` extractor would cause *Internal
/// Server Error* response.
///
/// ```rust
/// use std::sync::Mutex;
/// use actix_web::{web, App, HttpResponse, Responder};
///
/// struct MyData {
///     counter: usize,
/// }
///
/// /// Use the `Data<T>` extractor to access data in a handler.
/// async fn index(data: web::Data<Mutex<MyData>>) -> impl Responder {
///     let mut data = data.lock().unwrap();
///     data.counter += 1;
///     HttpResponse::Ok()
/// }
///
/// fn main() {
///     let data = web::Data::new(Mutex::new(MyData{ counter: 0 }));
///
///     let app = App::new()
///         // Store `MyData` in application storage.
///         .app_data(data.clone())
///         .service(
///             web::resource("/index.html").route(
///                 web::get().to(index)));
/// }
/// ```
#[derive(Debug)]
pub struct Data<T: ?Sized>(Arc<T>);

impl<T> Data<T> {
    /// Create new `Data` instance.
    pub fn new(state: T) -> Data<T> {
        Data(Arc::new(state))
    }

    /// Get reference to inner app data.
    pub fn get_ref(&self) -> &T {
        self.0.as_ref()
    }

    /// Convert to the internal Arc<T>
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
        Data(self.0.clone())
    }
}

impl<T: ?Sized> From<Arc<T>> for Data<T> {
    fn from(arc: Arc<T>) -> Self {
        Data(arc)
    }
}

impl<T: ?Sized + 'static> FromRequest for Data<T> {
    type Config = ();
    type Error = Error;
    type Future = Ready<Result<Self, Error>>;

    #[inline]
    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        if let Some(st) = req.app_data::<Data<T>>() {
            ok(st.clone())
        } else {
            log::debug!(
                "Failed to construct App-level Data extractor. \
                 Request path: {:?} (type: {})",
                req.path(),
                type_name::<T>(),
            );
            err(ErrorInternalServerError(
                "App data is not configured, to configure use App::data()",
            ))
        }
    }
}

impl<T: ?Sized + 'static> DataFactory for Data<T> {
    fn create(&self, extensions: &mut Extensions) -> bool {
        if !extensions.contains::<Data<T>>() {
            extensions.insert(Data(self.0.clone()));
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use actix_service::Service;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::http::StatusCode;
    use crate::test::{self, init_service, TestRequest};
    use crate::{web, App, HttpResponse};

    #[actix_rt::test]
    async fn test_data_extractor() {
        let mut srv = init_service(App::new().data("TEST".to_string()).service(
            web::resource("/").to(|data: web::Data<String>| {
                assert_eq!(data.to_lowercase(), "test");
                HttpResponse::Ok()
            }),
        ))
        .await;

        let req = TestRequest::default().to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let mut srv =
            init_service(App::new().data(10u32).service(
                web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok()),
            ))
            .await;
        let req = TestRequest::default().to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[actix_rt::test]
    async fn test_app_data_extractor() {
        let mut srv =
            init_service(App::new().app_data(Data::new(10usize)).service(
                web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok()),
            ))
            .await;

        let req = TestRequest::default().to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let mut srv =
            init_service(App::new().app_data(Data::new(10u32)).service(
                web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok()),
            ))
            .await;
        let req = TestRequest::default().to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[actix_rt::test]
    async fn test_route_data_extractor() {
        let mut srv = init_service(
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
        let mut srv = init_service(
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

    #[actix_rt::test]
    async fn test_override_data() {
        let mut srv = init_service(App::new().data(1usize).service(
            web::resource("/").data(10usize).route(web::get().to(
                |data: web::Data<usize>| {
                    assert_eq!(**data, 10);
                    HttpResponse::Ok()
                },
            )),
        ))
        .await;

        let req = TestRequest::default().to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_data_drop() {
        struct TestData(Arc<AtomicUsize>);

        impl TestData {
            fn new(inner: Arc<AtomicUsize>) -> Self {
                let _ = inner.fetch_add(1, Ordering::SeqCst);
                Self(inner)
            }
        }

        impl Clone for TestData {
            fn clone(&self) -> Self {
                let inner = self.0.clone();
                let _ = inner.fetch_add(1, Ordering::SeqCst);
                Self(inner)
            }
        }

        impl Drop for TestData {
            fn drop(&mut self) {
                let _ = self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }

        let num = Arc::new(AtomicUsize::new(0));
        let data = TestData::new(num.clone());
        assert_eq!(num.load(Ordering::SeqCst), 1);

        let srv = test::start(move || {
            let data = data.clone();

            App::new()
                .data(data)
                .service(web::resource("/").to(|_data: Data<TestData>| async { "ok" }))
        });

        assert!(srv.get("/").send().await.unwrap().status().is_success());
        srv.stop().await;

        assert_eq!(num.load(Ordering::SeqCst), 0);
    }

    #[actix_rt::test]
    async fn test_data_from_arc() {
        let data_new = Data::new(String::from("test-123"));
        let data_from_arc = Data::from(Arc::new(String::from("test-123")));
        assert_eq!(data_new.0, data_from_arc.0)
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
}
