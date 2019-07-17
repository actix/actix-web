//! Path extractor

use std::sync::Arc;
use std::{fmt, ops};

use actix_http::error::{Error, ErrorNotFound};
use actix_router::PathDeserializer;
use serde::de;

use crate::dev::Payload;
use crate::error::PathError;
use crate::request::HttpRequest;
use crate::FromRequest;

#[derive(PartialEq, Eq, PartialOrd, Ord)]
/// Extract typed information from the request's path.
///
/// ## Example
///
/// ```rust
/// use actix_web::{web, App};
///
/// /// extract path info from "/{username}/{count}/index.html" url
/// /// {username} - deserializes to a String
/// /// {count} -  - deserializes to a u32
/// fn index(info: web::Path<(String, u32)>) -> String {
///     format!("Welcome {}! {}", info.0, info.1)
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/{username}/{count}/index.html") // <- define path parameters
///              .route(web::get().to(index))               // <- register handler with `Path` extractor
///     );
/// }
/// ```
///
/// It is possible to extract path information to a specific type that
/// implements `Deserialize` trait from *serde*.
///
/// ```rust
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{web, App, Error};
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// /// extract `Info` from a path using serde
/// fn index(info: web::Path<Info>) -> Result<String, Error> {
///     Ok(format!("Welcome {}!", info.username))
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/{username}/index.html") // <- define path parameters
///              .route(web::get().to(index)) // <- use handler with Path` extractor
///     );
/// }
/// ```
pub struct Path<T> {
    inner: T,
}

impl<T> Path<T> {
    /// Deconstruct to an inner value
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T> AsRef<T> for Path<T> {
    fn as_ref(&self) -> &T {
        &self.inner
    }
}

impl<T> ops::Deref for Path<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T> ops::DerefMut for Path<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T> From<T> for Path<T> {
    fn from(inner: T) -> Path<T> {
        Path { inner }
    }
}

impl<T: fmt::Debug> fmt::Debug for Path<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl<T: fmt::Display> fmt::Display for Path<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.inner.fmt(f)
    }
}

/// Extract typed information from the request's path.
///
/// ## Example
///
/// ```rust
/// use actix_web::{web, App};
///
/// /// extract path info from "/{username}/{count}/index.html" url
/// /// {username} - deserializes to a String
/// /// {count} -  - deserializes to a u32
/// fn index(info: web::Path<(String, u32)>) -> String {
///     format!("Welcome {}! {}", info.0, info.1)
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/{username}/{count}/index.html") // <- define path parameters
///              .route(web::get().to(index)) // <- register handler with `Path` extractor
///     );
/// }
/// ```
///
/// It is possible to extract path information to a specific type that
/// implements `Deserialize` trait from *serde*.
///
/// ```rust
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{web, App, Error};
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// /// extract `Info` from a path using serde
/// fn index(info: web::Path<Info>) -> Result<String, Error> {
///     Ok(format!("Welcome {}!", info.username))
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/{username}/index.html") // <- define path parameters
///              .route(web::get().to(index)) // <- use handler with Path` extractor
///     );
/// }
/// ```
impl<T> FromRequest for Path<T>
where
    T: de::DeserializeOwned,
{
    type Error = Error;
    type Future = Result<Self, Error>;
    type Config = PathConfig;

    #[inline]
    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        let error_handler = req
            .app_data::<Self::Config>()
            .map(|c| c.ehandler.clone())
            .unwrap_or(None);

        de::Deserialize::deserialize(PathDeserializer::new(req.match_info()))
            .map(|inner| Path { inner })
            .map_err(move |e| {
                log::debug!(
                    "Failed during Path extractor deserialization. \
                     Request path: {:?}",
                    req.path()
                );
                if let Some(error_handler) = error_handler {
                    let e = PathError::Deserialize(e);
                    (error_handler)(e, req)
                } else {
                    ErrorNotFound(e)
                }
            })
    }
}

/// Path extractor configuration
///
/// ```rust
/// # #[macro_use]
/// # extern crate serde_derive;
/// use actix_web::web::PathConfig;
/// use actix_web::{error, web, App, FromRequest, HttpResponse};
///
/// #[derive(Deserialize, Debug)]
/// enum Folder {
///     #[serde(rename = "inbox")]
///     Inbox,
///     #[serde(rename = "outbox")]
///     Outbox,
/// }
///
/// // deserialize `Info` from request's path
/// fn index(folder: web::Path<Folder>) -> String {
///     format!("Selected folder: {:?}!", folder)
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/messages/{folder}")
///             .data(PathConfig::default().error_handler(|err, req| {
///                 error::InternalError::from_response(
///                     err,
///                     HttpResponse::Conflict().finish(),
///                 )
///                 .into()
///             }))
///             .route(web::post().to(index)),
///     );
/// }
/// ```
#[derive(Clone)]
pub struct PathConfig {
    ehandler: Option<Arc<dyn Fn(PathError, &HttpRequest) -> Error + Send + Sync>>,
}

impl PathConfig {
    /// Set custom error handler
    pub fn error_handler<F>(mut self, f: F) -> Self
    where
        F: Fn(PathError, &HttpRequest) -> Error + Send + Sync + 'static,
    {
        self.ehandler = Some(Arc::new(f));
        self
    }
}

impl Default for PathConfig {
    fn default() -> Self {
        PathConfig { ehandler: None }
    }
}

#[cfg(test)]
mod tests {
    use actix_router::ResourceDef;
    use derive_more::Display;
    use serde_derive::Deserialize;

    use super::*;
    use crate::test::{block_on, TestRequest};
    use crate::{error, http, HttpResponse};

    #[derive(Deserialize, Debug, Display)]
    #[display(fmt = "MyStruct({}, {})", key, value)]
    struct MyStruct {
        key: String,
        value: String,
    }

    #[derive(Deserialize)]
    struct Test2 {
        key: String,
        value: u32,
    }

    #[test]
    fn test_extract_path_single() {
        let resource = ResourceDef::new("/{value}/");

        let mut req = TestRequest::with_uri("/32/").to_srv_request();
        resource.match_path(req.match_info_mut());

        let (req, mut pl) = req.into_parts();
        assert_eq!(*Path::<i8>::from_request(&req, &mut pl).unwrap(), 32);
        assert!(Path::<MyStruct>::from_request(&req, &mut pl).is_err());
    }

    #[test]
    fn test_tuple_extract() {
        let resource = ResourceDef::new("/{key}/{value}/");

        let mut req = TestRequest::with_uri("/name/user1/?id=test").to_srv_request();
        resource.match_path(req.match_info_mut());

        let (req, mut pl) = req.into_parts();
        let res =
            block_on(<(Path<(String, String)>,)>::from_request(&req, &mut pl)).unwrap();
        assert_eq!((res.0).0, "name");
        assert_eq!((res.0).1, "user1");

        let res = block_on(
            <(Path<(String, String)>, Path<(String, String)>)>::from_request(
                &req, &mut pl,
            ),
        )
        .unwrap();
        assert_eq!((res.0).0, "name");
        assert_eq!((res.0).1, "user1");
        assert_eq!((res.1).0, "name");
        assert_eq!((res.1).1, "user1");

        let () = <()>::from_request(&req, &mut pl).unwrap();
    }

    #[test]
    fn test_request_extract() {
        let mut req = TestRequest::with_uri("/name/user1/?id=test").to_srv_request();

        let resource = ResourceDef::new("/{key}/{value}/");
        resource.match_path(req.match_info_mut());

        let (req, mut pl) = req.into_parts();
        let mut s = Path::<MyStruct>::from_request(&req, &mut pl).unwrap();
        assert_eq!(s.key, "name");
        assert_eq!(s.value, "user1");
        s.value = "user2".to_string();
        assert_eq!(s.value, "user2");
        assert_eq!(
            format!("{}, {:?}", s, s),
            "MyStruct(name, user2), MyStruct { key: \"name\", value: \"user2\" }"
        );
        let s = s.into_inner();
        assert_eq!(s.value, "user2");

        let s = Path::<(String, String)>::from_request(&req, &mut pl).unwrap();
        assert_eq!(s.0, "name");
        assert_eq!(s.1, "user1");

        let mut req = TestRequest::with_uri("/name/32/").to_srv_request();
        let resource = ResourceDef::new("/{key}/{value}/");
        resource.match_path(req.match_info_mut());

        let (req, mut pl) = req.into_parts();
        let s = Path::<Test2>::from_request(&req, &mut pl).unwrap();
        assert_eq!(s.as_ref().key, "name");
        assert_eq!(s.value, 32);

        let s = Path::<(String, u8)>::from_request(&req, &mut pl).unwrap();
        assert_eq!(s.0, "name");
        assert_eq!(s.1, 32);

        let res = Path::<Vec<String>>::from_request(&req, &mut pl).unwrap();
        assert_eq!(res[0], "name".to_owned());
        assert_eq!(res[1], "32".to_owned());
    }

    #[test]
    fn test_custom_err_handler() {
        let (req, mut pl) = TestRequest::with_uri("/name/user1/")
            .data(PathConfig::default().error_handler(|err, _| {
                error::InternalError::from_response(
                    err,
                    HttpResponse::Conflict().finish(),
                )
                .into()
            }))
            .to_http_parts();

        let s = block_on(Path::<(usize,)>::from_request(&req, &mut pl)).unwrap_err();
        let res: HttpResponse = s.into();

        assert_eq!(res.status(), http::StatusCode::CONFLICT);
    }
}
