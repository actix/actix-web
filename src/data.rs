use std::ops::Deref;
use std::sync::Arc;

use actix_http::error::{Error, ErrorInternalServerError};
use actix_http::Extensions;

use crate::dev::Payload;
use crate::extract::FromRequest;
use crate::request::HttpRequest;

/// Application data factory
pub(crate) trait DataFactory {
    fn create(&self, extensions: &mut Extensions) -> bool;
}

/// AppData is a trait generic over `Data<T>` and `DataRaw<T>`.
/// A type impl this trait can use `FromRequest` and `DataFactory` to extract and push data to extension
pub trait AppData {
    type Data;
    type Inner;
    /// Create new `Data` instance.
    fn new(state: Self::Data) -> Self;

    /// Get reference to inner app data.
    fn get_ref(&self) -> &Self::Data;

    /// Convert to the internal Data.
    fn into_inner(self) -> Self::Inner;
}

/// Application data.
///
/// Application data is an arbitrary data attached to the app.
/// Application data is available to all routes and could be added
/// during application configuration process
/// with `App::data()` method.
///
/// Application data could be accessed by using `Data<T>`
/// extractor where `T` is data type.
///
/// **Note**: http server accepts an application factory rather than
/// an application instance. Http server constructs an application
/// instance for each thread, thus application data must be constructed
/// multiple times. If you want to share data between different
/// threads, a shareable object should be used, e.g. `Send + Sync`. Application
/// data does not need to be `Send` or `Sync`. Internally `Data` type
/// uses `Arc`. if your data implements `Send` + `Sync` traits you can
/// use `web::Data::new()` and avoid double `Arc`.
///
/// If route data is not set for a handler, using `Data<T>` extractor would
/// cause *Internal Server Error* response.
///
/// ```rust
/// use std::sync::Mutex;
/// use actix_web::{web::{self, AppData}, App};
///
/// struct MyData {
///     counter: usize,
/// }
///
/// /// Use `Data<T>` extractor to access data in handler.
/// fn index(data: web::Data<Mutex<MyData>>) {
///     let mut data = data.lock().unwrap();
///     data.counter += 1;
/// }
///
/// fn main() {
///     let data = web::Data::new(Mutex::new(MyData{ counter: 0 }));
///
///     let app = App::new()
///         // Store `MyData` in application storage.
///         .register_data(data.clone())
///         .service(
///             web::resource("/index.html").route(
///                 web::get().to(index)));
/// }
/// ```
#[derive(Debug)]
pub struct Data<T>(Arc<T>);

impl<T> Deref for Data<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.0.as_ref()
    }
}

impl<T> Clone for Data<T> {
    fn clone(&self) -> Data<T> {
        Data(self.0.clone())
    }
}

impl<T> AppData for Data<T> {
    type Data = T;
    type Inner = Arc<T>;
    fn new(state: Self::Data) -> Self {
        Data(Arc::new(state))
    }

    fn get_ref(&self) -> &Self::Data {
        self.0.as_ref()
    }

    fn into_inner(self) -> Self::Inner {
        self.0
    }
}

/// Raw Application data.
///
/// Raw Application data shares the same principle of Application data.
/// The difference is Raw Application data explicitly require the data type to be `Send + Sync + Clone`.
///
/// This is useful when you introduce a foreign type(from other crates for example) that is already thread safe.
/// By using Raw Application data you can avoid the additional layer of `Arc` provided by `web::Data`
/// ```rust
/// use std::sync::{Arc, Mutex};
/// use actix_web::{web::{self, AppData}, App};
///
/// struct ForeignType {
///     inner: Arc<Mutex<usize>>
/// }
///
/// impl Clone for ForeignType {
///     fn clone(&self) -> Self {
///         ForeignType {
///             inner: self.inner.clone()
///         }
///     }
/// }
///
/// /// Use `DataRaw<T>` extractor to access data in handler.
/// fn index(data: web::DataRaw<ForeignType>) {
///     let mut data = data.inner.lock().unwrap();
///     *data += 1;
/// }
///
/// fn main() {
///     let data = ForeignType {
///         inner: Arc::new(Mutex::new(1usize))
///     };
///
///     let app = App::new()
///         // Store `ForeignTypeType` in application storage.
///         .data_raw(data.clone())
///         .service(
///             web::resource("/index.html").route(
///                 web::get().to(index)));
/// }
/// ```
#[derive(Debug)]
pub struct DataRaw<T: Send + Sync + Clone>(T);

impl<T: Send + Sync + Clone> Clone for DataRaw<T> {
    fn clone(&self) -> DataRaw<T> {
        DataRaw(self.0.clone())
    }
}

impl<T: Send + Sync + Clone> Deref for DataRaw<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> AppData for DataRaw<T>
    where T: Send + Sync + Clone {
    type Data = T;
    type Inner = T;
    fn new(state: Self::Data) -> Self {
        DataRaw(state)
    }

    fn get_ref(&self) -> &Self::Data {
        &*self
    }

    fn into_inner(self) -> Self::Inner {
        self.0
    }
}

impl<T: AppData + Clone + 'static> FromRequest for T {
    type Config = ();
    type Error = Error;
    type Future = Result<Self, Error>;

    #[inline]
    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        if let Some(st) = req.get_app_data::<T>() {
            Ok(st)
        } else {
            log::debug!(
                "Failed to construct App-level Data extractor. \
                 Request path: {:?}",
                req.path()
            );
            Err(ErrorInternalServerError(
                "App data is not configured, to configure use App::data()",
            ))
        }
    }
}

impl<T: AppData + Clone + 'static> DataFactory for T {
    fn create(&self, extensions: &mut Extensions) -> bool {
        if !extensions.contains::<T>() {
            extensions.insert(self.clone());
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, AtomicU32};

    use actix_service::Service;

    use super::*;
    use crate::http::StatusCode;
    use crate::test::{block_on, init_service, TestRequest};
    use crate::{web, App, HttpResponse};

    #[test]
    fn test_data_extractor() {
        let mut srv =
            init_service(App::new().data(10usize).service(
                web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok()),
            ));

        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let mut srv =
            init_service(App::new().data(10u32).service(
                web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok()),
            ));
        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_register_data_extractor() {
        let mut srv =
            init_service(App::new().register_data(Data::new(10usize)).service(
                web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok()),
            ));

        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let mut srv =
            init_service(App::new().register_data(Data::new(10u32)).service(
                web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok()),
            ));
        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_route_data_extractor() {
        let mut srv =
            init_service(App::new().service(web::resource("/").data(10usize).route(
                web::get().to(|data: web::Data<usize>| {
                    let _ = data.clone();
                    HttpResponse::Ok()
                }),
            )));

        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // different type
        let mut srv = init_service(
            App::new().service(
                web::resource("/")
                    .data(10u32)
                    .route(web::get().to(|_: web::Data<usize>| HttpResponse::Ok())),
            ),
        );
        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_override_data() {
        let mut srv = init_service(App::new().data(1usize).service(
            web::resource("/").data(10usize).route(web::get().to(
                |data: web::Data<usize>| {
                    assert_eq!(*data, 10);
                    let _ = data.clone();
                    HttpResponse::Ok()
                },
            )),
        ));

        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_data_raw_extractor() {
        let mut srv =
            init_service(App::new().data_raw(Arc::new(Mutex::new(1usize))).service(
                web::resource("/").to(|_: web::DataRaw<Arc<Mutex<usize>>>| HttpResponse::Ok()),
            ));

        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let mut srv =
            init_service(App::new().data_raw(Arc::new(AtomicUsize::new(1))).service(
                web::resource("/").to(|_: web::DataRaw<Arc<AtomicUsize>>| HttpResponse::Ok()),
            ));

        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let mut srv =
            init_service(App::new().data_raw(Arc::new(AtomicUsize::new(1))).service(
                web::resource("/").to(|_: web::DataRaw<Arc<AtomicU32>>| HttpResponse::Ok()),
            ));
        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_register_data_raw_extractor() {
        let mut srv =
            init_service(App::new().register_data(DataRaw::new(Arc::new(AtomicUsize::new(1)))).service(
                web::resource("/").to(|_: web::DataRaw<Arc<AtomicUsize>>| HttpResponse::Ok()),
            ));

        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let mut srv =
            init_service(App::new().register_data(DataRaw::new(Arc::new(AtomicUsize::new(1)))).service(
                web::resource("/").to(|_: web::DataRaw<Arc<AtomicU32>>| HttpResponse::Ok()),
            ));
        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

}
