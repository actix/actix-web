//! For path segment extractor documentation, see [`Path`].

use std::sync::Arc;

use actix_router::PathDeserializer;
use actix_utils::future::{ready, Ready};
use derive_more::{AsRef, Deref, DerefMut, Display, From};
use serde::de;

use crate::{
    dev::Payload,
    error::{Error, ErrorNotFound, PathError},
    web::Data,
    FromRequest, HttpRequest,
};

/// Extract typed data from request path segments.
///
/// Use [`PathConfig`] to configure extraction option.
///
/// Unlike, [`HttpRequest::match_info`], this extractor will fully percent-decode dynamic segments,
/// including `/`, `%`, and `+`.
///
/// # Examples
/// ```
/// use actix_web::{get, web};
///
/// // extract path info from "/{name}/{count}/index.html" into tuple
/// // {name}  - deserialize a String
/// // {count} - deserialize a u32
/// #[get("/{name}/{count}/index.html")]
/// async fn index(path: web::Path<(String, u32)>) -> String {
///     let (name, count) = path.into_inner();
///     format!("Welcome {}! {}", name, count)
/// }
/// ```
///
/// Path segments also can be deserialized into any type that implements [`serde::Deserialize`].
/// Path segment labels will be matched with struct field names.
///
/// ```
/// use actix_web::{get, web};
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct Info {
///     name: String,
/// }
///
/// // extract `Info` from a path using serde
/// #[get("/{name}")]
/// async fn index(info: web::Path<Info>) -> String {
///     format!("Welcome {}!", info.name)
/// }
/// ```
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Deref, DerefMut, AsRef, Display, From)]
pub struct Path<T>(T);

impl<T> Path<T> {
    /// Unwrap into inner `T` value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

/// See [here](#Examples) for example of usage as an extractor.
impl<T> FromRequest for Path<T>
where
    T: de::DeserializeOwned,
{
    type Error = Error;
    type Future = Ready<Result<Self, Self::Error>>;

    #[inline]
    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        let error_handler = req
            .app_data::<PathConfig>()
            .or_else(|| req.app_data::<Data<PathConfig>>().map(Data::get_ref))
            .and_then(|c| c.err_handler.clone());

        ready(
            de::Deserialize::deserialize(PathDeserializer::new(req.match_info()))
                .map(Path)
                .map_err(move |err| {
                    log::debug!(
                        "Failed during Path extractor deserialization. \
                         Request path: {:?}",
                        req.path()
                    );

                    if let Some(error_handler) = error_handler {
                        let e = PathError::Deserialize(err);
                        (error_handler)(e, req)
                    } else {
                        ErrorNotFound(err)
                    }
                }),
        )
    }
}

/// Path extractor configuration
///
/// ```
/// use actix_web::web::PathConfig;
/// use actix_web::{error, web, App, FromRequest, HttpResponse};
/// use serde::Deserialize;
///
/// #[derive(Deserialize, Debug)]
/// enum Folder {
///     #[serde(rename = "inbox")]
///     Inbox,
///
///     #[serde(rename = "outbox")]
///     Outbox,
/// }
///
/// // deserialize `Info` from request's path
/// async fn index(folder: web::Path<Folder>) -> String {
///     format!("Selected folder: {:?}!", folder)
/// }
///
/// let app = App::new().service(
///     web::resource("/messages/{folder}")
///         .app_data(PathConfig::default().error_handler(|err, req| {
///             error::InternalError::from_response(
///                 err,
///                 HttpResponse::Conflict().into(),
///             )
///             .into()
///         }))
///         .route(web::post().to(index)),
/// );
/// ```
#[derive(Clone, Default)]
pub struct PathConfig {
    #[allow(clippy::type_complexity)]
    err_handler: Option<Arc<dyn Fn(PathError, &HttpRequest) -> Error + Send + Sync>>,
}

impl PathConfig {
    /// Set custom error handler.
    pub fn error_handler<F>(mut self, f: F) -> Self
    where
        F: Fn(PathError, &HttpRequest) -> Error + Send + Sync + 'static,
    {
        self.err_handler = Some(Arc::new(f));
        self
    }
}

#[cfg(test)]
mod tests {
    use actix_router::ResourceDef;
    use derive_more::Display;
    use serde::Deserialize;

    use super::*;
    use crate::{error, http, test::TestRequest, HttpResponse};

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

    #[actix_rt::test]
    async fn test_extract_path_single() {
        let resource = ResourceDef::new("/{value}/");

        let mut req = TestRequest::with_uri("/32/").to_srv_request();
        resource.capture_match_info(req.match_info_mut());

        let (req, mut pl) = req.into_parts();
        assert_eq!(*Path::<i8>::from_request(&req, &mut pl).await.unwrap(), 32);
        assert!(Path::<MyStruct>::from_request(&req, &mut pl).await.is_err());
    }

    #[allow(clippy::let_unit_value)]
    #[actix_rt::test]
    async fn test_tuple_extract() {
        let resource = ResourceDef::new("/{key}/{value}/");

        let mut req = TestRequest::with_uri("/name/user1/?id=test").to_srv_request();
        resource.capture_match_info(req.match_info_mut());

        let (req, mut pl) = req.into_parts();
        let (Path(res),) = <(Path<(String, String)>,)>::from_request(&req, &mut pl)
            .await
            .unwrap();
        assert_eq!(res.0, "name");
        assert_eq!(res.1, "user1");

        let (Path(a), Path(b)) =
            <(Path<(String, String)>, Path<(String, String)>)>::from_request(&req, &mut pl)
                .await
                .unwrap();
        assert_eq!(a.0, "name");
        assert_eq!(a.1, "user1");
        assert_eq!(b.0, "name");
        assert_eq!(b.1, "user1");

        let () = <()>::from_request(&req, &mut pl).await.unwrap();
    }

    #[actix_rt::test]
    async fn test_request_extract() {
        let mut req = TestRequest::with_uri("/name/user1/?id=test").to_srv_request();

        let resource = ResourceDef::new("/{key}/{value}/");
        resource.capture_match_info(req.match_info_mut());

        let (req, mut pl) = req.into_parts();
        let mut s = Path::<MyStruct>::from_request(&req, &mut pl).await.unwrap();
        assert_eq!(s.key, "name");
        assert_eq!(s.value, "user1");
        s.value = "user2".to_string();
        assert_eq!(s.value, "user2");
        assert_eq!(
            format!("{}, {:?}", s, s),
            "MyStruct(name, user2), Path(MyStruct { key: \"name\", value: \"user2\" })"
        );
        let s = s.into_inner();
        assert_eq!(s.value, "user2");

        let Path(s) = Path::<(String, String)>::from_request(&req, &mut pl)
            .await
            .unwrap();
        assert_eq!(s.0, "name");
        assert_eq!(s.1, "user1");

        let mut req = TestRequest::with_uri("/name/32/").to_srv_request();
        let resource = ResourceDef::new("/{key}/{value}/");
        resource.capture_match_info(req.match_info_mut());

        let (req, mut pl) = req.into_parts();
        let s = Path::<Test2>::from_request(&req, &mut pl).await.unwrap();
        assert_eq!(s.as_ref().key, "name");
        assert_eq!(s.value, 32);

        let Path(s) = Path::<(String, u8)>::from_request(&req, &mut pl)
            .await
            .unwrap();
        assert_eq!(s.0, "name");
        assert_eq!(s.1, 32);

        let res = Path::<Vec<String>>::from_request(&req, &mut pl)
            .await
            .unwrap();
        assert_eq!(res[0], "name".to_owned());
        assert_eq!(res[1], "32".to_owned());
    }

    #[actix_rt::test]
    async fn paths_decoded() {
        let resource = ResourceDef::new("/{key}/{value}");
        let mut req = TestRequest::with_uri("/na%2Bme/us%2Fer%254%32").to_srv_request();
        resource.capture_match_info(req.match_info_mut());

        let (req, mut pl) = req.into_parts();
        let path_items = Path::<MyStruct>::from_request(&req, &mut pl).await.unwrap();
        assert_eq!(path_items.key, "na+me");
        assert_eq!(path_items.value, "us/er%42");
        assert_eq!(req.match_info().as_str(), "/na%2Bme/us%2Fer%2542");
    }

    #[actix_rt::test]
    async fn test_custom_err_handler() {
        let (req, mut pl) = TestRequest::with_uri("/name/user1/")
            .app_data(PathConfig::default().error_handler(|err, _| {
                error::InternalError::from_response(err, HttpResponse::Conflict().finish()).into()
            }))
            .to_http_parts();

        let s = Path::<(usize,)>::from_request(&req, &mut pl)
            .await
            .unwrap_err();
        let res = HttpResponse::from_error(s);

        assert_eq!(res.status(), http::StatusCode::CONFLICT);
    }
}
