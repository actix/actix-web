#![allow(dead_code)]
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use std::{fmt, str};

use bytes::Bytes;
use encoding::all::UTF_8;
use encoding::types::{DecoderTrap, Encoding};
use futures::future::{err, ok, Either, FutureResult};
use futures::{future, Async, Future, IntoFuture, Poll, Stream};
use mime::Mime;
use serde::de::{self, DeserializeOwned};
use serde::Serialize;
use serde_json;
use serde_urlencoded;

use actix_http::dev::{JsonBody, MessageBody, UrlEncoded};
use actix_http::error::{
    Error, ErrorBadRequest, ErrorNotFound, JsonPayloadError, PayloadError,
    UrlencodedError,
};
use actix_http::http::StatusCode;
use actix_http::{Extensions, HttpMessage, Response};
use actix_router::PathDeserializer;

use crate::request::HttpRequest;
use crate::responder::Responder;
use crate::service::ServiceFromRequest;

/// Trait implemented by types that can be extracted from request.
///
/// Types that implement this trait can be used with `Route` handlers.
pub trait FromRequest<P>: Sized {
    /// The associated error which can be returned.
    type Error: Into<Error>;

    /// Future that resolves to a Self
    type Future: IntoFuture<Item = Self, Error = Self::Error>;

    /// Configuration for the extractor
    type Config: ExtractorConfig;

    /// Convert request to a Self
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future;
}

/// Storage for extractor configs
#[derive(Default)]
pub struct ConfigStorage {
    pub(crate) storage: Option<Rc<Extensions>>,
}

impl ConfigStorage {
    pub fn store<C: ExtractorConfig>(&mut self, config: C) {
        if self.storage.is_none() {
            self.storage = Some(Rc::new(Extensions::new()));
        }
        if let Some(ref mut ext) = self.storage {
            Rc::get_mut(ext).unwrap().insert(config);
        }
    }
}

pub trait ExtractorConfig: Default + Clone + 'static {
    /// Set default configuration to config storage
    fn store_default(ext: &mut ConfigStorage) {
        ext.store(Self::default())
    }
}

impl ExtractorConfig for () {
    fn store_default(_: &mut ConfigStorage) {}
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
/// Extract typed information from the request's path.
///
/// ## Example
///
/// ```rust
/// use actix_web::{web, http, App, extract::Path};
///
/// /// extract path info from "/{username}/{count}/index.html" url
/// /// {username} - deserializes to a String
/// /// {count} -  - deserializes to a u32
/// fn index(info: Path<(String, u32)>) -> String {
///     format!("Welcome {}! {}", info.0, info.1)
/// }
///
/// fn main() {
///     let app = App::new().resource(
///         "/{username}/{count}/index.html", // <- define path parameters
///         |r| r.route(web::get().to(index)) // <- register handler with `Path` extractor
///     );
/// }
/// ```
///
/// It is possible to extract path information to a specific type that
/// implements `Deserialize` trait from *serde*.
///
/// ```rust
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{web, App, extract::Path, Error};
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// /// extract `Info` from a path using serde
/// fn index(info: Path<Info>) -> Result<String, Error> {
///     Ok(format!("Welcome {}!", info.username))
/// }
///
/// fn main() {
///     let app = App::new().resource(
///         "/{username}/index.html", // <- define path parameters
///         |r| r.route(web::get().to(index)) // <- use handler with Path` extractor
///     );
/// }
/// ```
pub struct Path<T> {
    inner: T,
}

impl<T> AsRef<T> for Path<T> {
    fn as_ref(&self) -> &T {
        &self.inner
    }
}

impl<T> Deref for Path<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T> DerefMut for Path<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T> Path<T> {
    /// Deconstruct to an inner value
    pub fn into_inner(self) -> T {
        self.inner
    }

    /// Extract path information from a request
    pub fn extract(req: &HttpRequest) -> Result<Path<T>, de::value::Error>
    where
        T: DeserializeOwned,
    {
        de::Deserialize::deserialize(PathDeserializer::new(req.match_info()))
            .map(|inner| Path { inner })
    }
}

impl<T> From<T> for Path<T> {
    fn from(inner: T) -> Path<T> {
        Path { inner }
    }
}

/// Extract typed information from the request's path.
///
/// ## Example
///
/// ```rust
/// use actix_web::{web, http, App, extract::Path};
///
/// /// extract path info from "/{username}/{count}/index.html" url
/// /// {username} - deserializes to a String
/// /// {count} -  - deserializes to a u32
/// fn index(info: Path<(String, u32)>) -> String {
///     format!("Welcome {}! {}", info.0, info.1)
/// }
///
/// fn main() {
///     let app = App::new().resource(
///         "/{username}/{count}/index.html", // <- define path parameters
///         |r| r.route(web::get().to(index)) // <- register handler with `Path` extractor
///     );
/// }
/// ```
///
/// It is possible to extract path information to a specific type that
/// implements `Deserialize` trait from *serde*.
///
/// ```rust
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{web, App, extract::Path, Error};
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// /// extract `Info` from a path using serde
/// fn index(info: Path<Info>) -> Result<String, Error> {
///     Ok(format!("Welcome {}!", info.username))
/// }
///
/// fn main() {
///     let app = App::new().resource(
///         "/{username}/index.html", // <- define path parameters
///         |r| r.route(web::get().to(index)) // <- use handler with Path` extractor
///     );
/// }
/// ```
impl<T, P> FromRequest<P> for Path<T>
where
    T: DeserializeOwned,
{
    type Error = Error;
    type Future = Result<Self, Error>;
    type Config = ();

    #[inline]
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        Self::extract(req).map_err(ErrorNotFound)
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

#[derive(PartialEq, Eq, PartialOrd, Ord)]
/// Extract typed information from from the request's query.
///
/// ## Example
///
/// ```rust
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{web, extract, App};
///
/// #[derive(Debug, Deserialize)]
/// pub enum ResponseType {
///    Token,
///    Code
/// }
///
/// #[derive(Deserialize)]
/// pub struct AuthRequest {
///    id: u64,
///    response_type: ResponseType,
/// }
///
/// // Use `Query` extractor for query information.
/// // This handler get called only if request's query contains `username` field
/// // The correct request for this handler would be `/index.html?id=64&response_type=Code"`
/// fn index(info: extract::Query<AuthRequest>) -> String {
///     format!("Authorization request for client with id={} and type={:?}!", info.id, info.response_type)
/// }
///
/// fn main() {
///     let app = App::new().resource(
///        "/index.html",
///        |r| r.route(web::get().to(index))); // <- use `Query` extractor
/// }
/// ```
pub struct Query<T>(T);

impl<T> Deref for Query<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Query<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T> Query<T> {
    /// Deconstruct to a inner value
    pub fn into_inner(self) -> T {
        self.0
    }
}

/// Extract typed information from from the request's query.
///
/// ## Example
///
/// ```rust
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{web, extract, App};
///
/// #[derive(Debug, Deserialize)]
/// pub enum ResponseType {
///    Token,
///    Code
/// }
///
/// #[derive(Deserialize)]
/// pub struct AuthRequest {
///    id: u64,
///    response_type: ResponseType,
/// }
///
/// // Use `Query` extractor for query information.
/// // This handler get called only if request's query contains `username` field
/// // The correct request for this handler would be `/index.html?id=64&response_type=Code"`
/// fn index(info: extract::Query<AuthRequest>) -> String {
///     format!("Authorization request for client with id={} and type={:?}!", info.id, info.response_type)
/// }
///
/// fn main() {
///     let app = App::new().resource(
///        "/index.html",
///        |r| r.route(web::get().to(index))); // <- use `Query` extractor
/// }
/// ```
impl<T, P> FromRequest<P> for Query<T>
where
    T: de::DeserializeOwned,
{
    type Error = Error;
    type Future = Result<Self, Error>;
    type Config = ();

    #[inline]
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        serde_urlencoded::from_str::<T>(req.query_string())
            .map(|val| Ok(Query(val)))
            .unwrap_or_else(|e| Err(e.into()))
    }
}

impl<T: fmt::Debug> fmt::Debug for Query<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<T: fmt::Display> fmt::Display for Query<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
/// Extract typed information from the request's body.
///
/// To extract typed information from request's body, the type `T` must
/// implement the `Deserialize` trait from *serde*.
///
/// [**FormConfig**](struct.FormConfig.html) allows to configure extraction
/// process.
///
/// ## Example
///
/// ```rust
/// # extern crate actix_web;
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{web, App, extract::Form};
///
/// #[derive(Deserialize)]
/// struct FormData {
///     username: String,
/// }
///
/// /// Extract form data using serde.
/// /// This handler get called only if content type is *x-www-form-urlencoded*
/// /// and content of the request could be deserialized to a `FormData` struct
/// fn index(form: Form<FormData>) -> String {
///     format!("Welcome {}!", form.username)
/// }
/// # fn main() {}
/// ```
pub struct Form<T>(pub T);

impl<T> Form<T> {
    /// Deconstruct to an inner value
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for Form<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Form<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T, P> FromRequest<P> for Form<T>
where
    T: DeserializeOwned + 'static,
    P: Stream<Item = Bytes, Error = PayloadError> + 'static,
{
    type Error = Error;
    type Future = Box<Future<Item = Self, Error = Error>>;
    type Config = FormConfig;

    #[inline]
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        let req2 = req.clone();
        let cfg = req.load_config::<FormConfig>();

        let limit = cfg.limit;
        let err = Rc::clone(&cfg.ehandler);
        Box::new(
            UrlEncoded::new(req)
                .limit(limit)
                .map_err(move |e| (*err)(e, &req2))
                .map(Form),
        )
    }
}

impl<T: fmt::Debug> fmt::Debug for Form<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<T: fmt::Display> fmt::Display for Form<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Form extractor configuration
///
/// ```rust
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{web, extract, App, Result};
///
/// #[derive(Deserialize)]
/// struct FormData {
///     username: String,
/// }
///
/// /// Extract form data using serde.
/// /// Custom configuration is used for this handler, max payload size is 4k
/// fn index(form: extract::Form<FormData>) -> Result<String> {
///     Ok(format!("Welcome {}!", form.username))
/// }
///
/// fn main() {
///     let app = App::new().resource(
///         "/index.html",
///         |r| {
///             r.route(web::get()
///                 // change `Form` extractor configuration
///                 .config(extract::FormConfig::default().limit(4097))
///                 .to(index))
///         });
/// }
/// ```
#[derive(Clone)]
pub struct FormConfig {
    limit: usize,
    ehandler: Rc<Fn(UrlencodedError, &HttpRequest) -> Error>,
}

impl FormConfig {
    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Set custom error handler
    pub fn error_handler<F>(mut self, f: F) -> Self
    where
        F: Fn(UrlencodedError, &HttpRequest) -> Error + 'static,
    {
        self.ehandler = Rc::new(f);
        self
    }
}

impl ExtractorConfig for FormConfig {}

impl Default for FormConfig {
    fn default() -> Self {
        FormConfig {
            limit: 262_144,
            ehandler: Rc::new(|e, _| e.into()),
        }
    }
}

/// Json helper
///
/// Json can be used for two different purpose. First is for json response
/// generation and second is for extracting typed information from request's
/// payload.
///
/// To extract typed information from request's body, the type `T` must
/// implement the `Deserialize` trait from *serde*.
///
/// [**JsonConfig**](struct.JsonConfig.html) allows to configure extraction
/// process.
///
/// ## Example
///
/// ```rust
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{web, extract, App};
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// /// deserialize `Info` from request's body
/// fn index(info: extract::Json<Info>) -> String {
///     format!("Welcome {}!", info.username)
/// }
///
/// fn main() {
///     let app = App::new().resource(
///        "/index.html",
///        |r| r.route(web::post().to(index)));
/// }
/// ```
///
/// The `Json` type allows you to respond with well-formed JSON data: simply
/// return a value of type Json<T> where T is the type of a structure
/// to serialize into *JSON*. The type `T` must implement the `Serialize`
/// trait from *serde*.
///
/// ```rust
/// # #[macro_use] extern crate serde_derive;
/// # use actix_web::*;
/// #
/// #[derive(Serialize)]
/// struct MyObj {
///     name: String,
/// }
///
/// fn index(req: HttpRequest) -> Result<Json<MyObj>> {
///     Ok(Json(MyObj {
///         name: req.match_info().get("name").unwrap().to_string(),
///     }))
/// }
/// # fn main() {}
/// ```
pub struct Json<T>(pub T);

impl<T> Json<T> {
    /// Deconstruct to an inner value
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for Json<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Json<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T> fmt::Debug for Json<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Json: {:?}", self.0)
    }
}

impl<T> fmt::Display for Json<T>
where
    T: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl<T: Serialize> Responder for Json<T> {
    type Error = Error;
    type Future = Result<Response, Error>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        let body = match serde_json::to_string(&self.0) {
            Ok(body) => body,
            Err(e) => return Err(e.into()),
        };

        Ok(Response::build(StatusCode::OK)
            .content_type("application/json")
            .body(body))
    }
}

/// Json extractor. Allow to extract typed information from request's
/// payload.
///
/// To extract typed information from request's body, the type `T` must
/// implement the `Deserialize` trait from *serde*.
///
/// [**JsonConfig**](struct.JsonConfig.html) allows to configure extraction
/// process.
///
/// ## Example
///
/// ```rust
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{web, extract, App};
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// /// deserialize `Info` from request's body
/// fn index(info: extract::Json<Info>) -> String {
///     format!("Welcome {}!", info.username)
/// }
///
/// fn main() {
///     let app = App::new().resource(
///        "/index.html",
///        |r| r.route(web::post().to(index)));
/// }
/// ```
impl<T, P> FromRequest<P> for Json<T>
where
    T: DeserializeOwned + 'static,
    P: Stream<Item = Bytes, Error = PayloadError> + 'static,
{
    type Error = Error;
    type Future = Box<Future<Item = Self, Error = Error>>;
    type Config = JsonConfig;

    #[inline]
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        let req2 = req.clone();
        let cfg = req.load_config::<JsonConfig>();

        let limit = cfg.limit;
        let err = Rc::clone(&cfg.ehandler);
        Box::new(
            JsonBody::new(req)
                .limit(limit)
                .map_err(move |e| (*err)(e, &req2))
                .map(Json),
        )
    }
}

/// Json extractor configuration
///
/// ```rust
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{error, extract, web, App, HttpResponse, Json};
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// /// deserialize `Info` from request's body, max payload size is 4kb
/// fn index(info: Json<Info>) -> String {
///     format!("Welcome {}!", info.username)
/// }
///
/// fn main() {
///     let app = App::new().resource("/index.html", |r| {
///         r.route(web::post().config(
///             // change json extractor configuration
///             extract::JsonConfig::default().limit(4096)
///                 .error_handler(|err, req| {  // <- create custom error response
///                     error::InternalError::from_response(
///                         err, HttpResponse::Conflict().finish()).into()
///                 }))
///             .to(index))
///     });
/// }
/// ```
#[derive(Clone)]
pub struct JsonConfig {
    limit: usize,
    ehandler: Rc<Fn(JsonPayloadError, &HttpRequest) -> Error>,
}

impl JsonConfig {
    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Set custom error handler
    pub fn error_handler<F>(mut self, f: F) -> Self
    where
        F: Fn(JsonPayloadError, &HttpRequest) -> Error + 'static,
    {
        self.ehandler = Rc::new(f);
        self
    }
}

impl ExtractorConfig for JsonConfig {}

impl Default for JsonConfig {
    fn default() -> Self {
        JsonConfig {
            limit: 262_144,
            ehandler: Rc::new(|e, _| e.into()),
        }
    }
}

/// Request binary data from a request's payload.
///
/// Loads request's payload and construct Bytes instance.
///
/// [**PayloadConfig**](struct.PayloadConfig.html) allows to configure
/// extraction process.
///
/// ## Example
///
/// ```rust
/// use bytes::Bytes;
/// use actix_web::{web, App};
///
/// /// extract binary data from request
/// fn index(body: Bytes) -> String {
///     format!("Body {:?}!", body)
/// }
///
/// fn main() {
///     let app = App::new()
///         .resource("/index.html", |r| r.route(web::get().to(index)));
/// }
/// ```
impl<P> FromRequest<P> for Bytes
where
    P: Stream<Item = Bytes, Error = PayloadError> + 'static,
{
    type Error = Error;
    type Future =
        Either<Box<Future<Item = Bytes, Error = Error>>, FutureResult<Bytes, Error>>;
    type Config = PayloadConfig;

    #[inline]
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        let cfg = req.load_config::<PayloadConfig>();

        if let Err(e) = cfg.check_mimetype(req) {
            return Either::B(err(e));
        }

        let limit = cfg.limit;
        Either::A(Box::new(MessageBody::new(req).limit(limit).from_err()))
    }
}

/// Extract text information from a request's body.
///
/// Text extractor automatically decode body according to the request's charset.
///
/// [**PayloadConfig**](struct.PayloadConfig.html) allows to configure
/// extraction process.
///
/// ## Example
///
/// ```rust
/// use actix_web::{web, extract, App};
///
/// /// extract text data from request
/// fn index(text: String) -> String {
///     format!("Body {}!", text)
/// }
///
/// fn main() {
///     let app = App::new().resource("/index.html", |r| {
///         r.route(
///             web::get()
///                .config(extract::PayloadConfig::new(4096)) // <- limit size of the payload
///                .to(index))  // <- register handler with extractor params
///     });
/// }
/// ```
impl<P> FromRequest<P> for String
where
    P: Stream<Item = Bytes, Error = PayloadError> + 'static,
{
    type Error = Error;
    type Future =
        Either<Box<Future<Item = String, Error = Error>>, FutureResult<String, Error>>;
    type Config = PayloadConfig;

    #[inline]
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        let cfg = req.load_config::<PayloadConfig>();

        // check content-type
        if let Err(e) = cfg.check_mimetype(req) {
            return Either::B(err(e));
        }

        // check charset
        let encoding = match req.encoding() {
            Ok(enc) => enc,
            Err(e) => return Either::B(err(e.into())),
        };
        let limit = cfg.limit;

        Either::A(Box::new(
            MessageBody::new(req)
                .limit(limit)
                .from_err()
                .and_then(move |body| {
                    let enc: *const Encoding = encoding as *const Encoding;
                    if enc == UTF_8 {
                        Ok(str::from_utf8(body.as_ref())
                            .map_err(|_| ErrorBadRequest("Can not decode body"))?
                            .to_owned())
                    } else {
                        Ok(encoding
                            .decode(&body, DecoderTrap::Strict)
                            .map_err(|_| ErrorBadRequest("Can not decode body"))?)
                    }
                }),
        ))
    }
}

/// Optionally extract a field from the request
///
/// If the FromRequest for T fails, return None rather than returning an error response
///
/// ## Example
///
/// ```rust
/// # #[macro_use] extern crate serde_derive;
/// use actix_web::{web, App, Error, FromRequest, ServiceFromRequest};
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
///     type Config = ();
///
///     fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
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
///     let app = App::new().resource("/users/:first", |r| {
///         r.route(web::post().to(index))
///     });
/// }
/// ```
impl<T: 'static, P> FromRequest<P> for Option<T>
where
    T: FromRequest<P>,
    T::Future: 'static,
{
    type Error = Error;
    type Future = Box<Future<Item = Option<T>, Error = Error>>;
    type Config = T::Config;

    #[inline]
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        Box::new(T::from_request(req).into_future().then(|r| match r {
            Ok(v) => future::ok(Some(v)),
            Err(_) => future::ok(None),
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
/// use actix_web::{web, App, Result, Error, FromRequest, ServiceFromRequest};
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
///     type Config = ();
///
///     fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
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
///     let app = App::new().resource("/users/:first", |r| {
///         r.route(web::post().to(index))
///     });
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
    type Config = T::Config;

    #[inline]
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        Box::new(T::from_request(req).into_future().then(|res| match res {
            Ok(v) => ok(Ok(v)),
            Err(e) => ok(Err(e)),
        }))
    }
}

/// Payload configuration for request's payload.
#[derive(Clone)]
pub struct PayloadConfig {
    limit: usize,
    mimetype: Option<Mime>,
}

impl PayloadConfig {
    /// Create `PayloadConfig` instance and set max size of payload.
    pub fn new(limit: usize) -> Self {
        Self::default().limit(limit)
    }

    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Set required mime-type of the request. By default mime type is not
    /// enforced.
    pub fn mimetype(mut self, mt: Mime) -> Self {
        self.mimetype = Some(mt);
        self
    }

    fn check_mimetype<P>(&self, req: &ServiceFromRequest<P>) -> Result<(), Error> {
        // check content-type
        if let Some(ref mt) = self.mimetype {
            match req.mime_type() {
                Ok(Some(ref req_mt)) => {
                    if mt != req_mt {
                        return Err(ErrorBadRequest("Unexpected Content-Type"));
                    }
                }
                Ok(None) => {
                    return Err(ErrorBadRequest("Content-Type is expected"));
                }
                Err(err) => {
                    return Err(err.into());
                }
            }
        }
        Ok(())
    }
}

impl ExtractorConfig for PayloadConfig {}

impl Default for PayloadConfig {
    fn default() -> Self {
        PayloadConfig {
            limit: 262_144,
            mimetype: None,
        }
    }
}

#[doc(hidden)]
impl<P> FromRequest<P> for () {
    type Error = Error;
    type Future = Result<(), Error>;
    type Config = ();

    fn from_request(_req: &mut ServiceFromRequest<P>) -> Self::Future {
        Ok(())
    }
}

macro_rules! tuple_config ({ $($T:ident),+} => {
    impl<$($T,)+> ExtractorConfig for ($($T,)+)
    where $($T: ExtractorConfig + Clone,)+
    {
        fn store_default(ext: &mut ConfigStorage) {
            $($T::store_default(ext);)+
        }
    }
});

macro_rules! tuple_from_req ({$fut_type:ident, $(($n:tt, $T:ident)),+} => {

    /// FromRequest implementation for tuple
    #[doc(hidden)]
    impl<P, $($T: FromRequest<P> + 'static),+> FromRequest<P> for ($($T,)+)
    {
        type Error = Error;
        type Future = $fut_type<P, $($T),+>;
        type Config = ($($T::Config,)+);

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

tuple_config!(A);
tuple_config!(A, B);
tuple_config!(A, B, C);
tuple_config!(A, B, C, D);
tuple_config!(A, B, C, D, E);
tuple_config!(A, B, C, D, E, F);
tuple_config!(A, B, C, D, E, F, G);
tuple_config!(A, B, C, D, E, F, G, H);
tuple_config!(A, B, C, D, E, F, G, H, I);
tuple_config!(A, B, C, D, E, F, G, H, I, J);

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
    use crate::test::TestRequest;

    #[derive(Deserialize, Debug, PartialEq)]
    struct Info {
        hello: String,
    }

    #[test]
    fn test_bytes() {
        let mut rt = actix_rt::Runtime::new().unwrap();
        let mut req = TestRequest::with_header(header::CONTENT_LENGTH, "11")
            .set_payload(Bytes::from_static(b"hello=world"))
            .to_from();

        let s = rt.block_on(Bytes::from_request(&mut req)).unwrap();
        assert_eq!(s, Bytes::from_static(b"hello=world"));
    }

    #[test]
    fn test_string() {
        let mut rt = actix_rt::Runtime::new().unwrap();
        let mut req = TestRequest::with_header(header::CONTENT_LENGTH, "11")
            .set_payload(Bytes::from_static(b"hello=world"))
            .to_from();

        let s = rt.block_on(String::from_request(&mut req)).unwrap();
        assert_eq!(s, "hello=world");
    }

    #[test]
    fn test_form() {
        let mut rt = actix_rt::Runtime::new().unwrap();
        let mut req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(header::CONTENT_LENGTH, "11")
        .set_payload(Bytes::from_static(b"hello=world"))
        .to_from();

        let s = rt.block_on(Form::<Info>::from_request(&mut req)).unwrap();
        assert_eq!(s.hello, "world");
    }

    #[test]
    fn test_option() {
        let mut rt = actix_rt::Runtime::new().unwrap();
        let mut req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .config(FormConfig::default().limit(4096))
        .to_from();

        let r = rt
            .block_on(Option::<Form<Info>>::from_request(&mut req))
            .unwrap();
        assert_eq!(r, None);

        let mut req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(header::CONTENT_LENGTH, "9")
        .set_payload(Bytes::from_static(b"hello=world"))
        .to_from();

        let r = rt
            .block_on(Option::<Form<Info>>::from_request(&mut req))
            .unwrap();
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

        let r = rt
            .block_on(Option::<Form<Info>>::from_request(&mut req))
            .unwrap();
        assert_eq!(r, None);
    }

    #[test]
    fn test_result() {
        let mut rt = actix_rt::Runtime::new().unwrap();
        let mut req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(header::CONTENT_LENGTH, "11")
        .set_payload(Bytes::from_static(b"hello=world"))
        .to_from();

        let r = rt
            .block_on(Result::<Form<Info>, Error>::from_request(&mut req))
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

        let r = rt
            .block_on(Result::<Form<Info>, Error>::from_request(&mut req))
            .unwrap();
        assert!(r.is_err());
    }

    #[test]
    fn test_payload_config() {
        let req = TestRequest::default().to_from();
        let cfg = PayloadConfig::default().mimetype(mime::APPLICATION_JSON);
        assert!(cfg.check_mimetype(&req).is_err());

        let req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .to_from();
        assert!(cfg.check_mimetype(&req).is_err());

        let req =
            TestRequest::with_header(header::CONTENT_TYPE, "application/json").to_from();
        assert!(cfg.check_mimetype(&req).is_ok());
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

    #[test]
    fn test_extract_path_single() {
        let resource = ResourceDef::new("/{value}/");

        let mut req = TestRequest::with_uri("/32/").to_from();
        resource.match_path(req.match_info_mut());

        assert_eq!(*Path::<i8>::from_request(&mut req).unwrap(), 32);
    }

    #[test]
    fn test_tuple_extract() {
        let mut rt = actix_rt::Runtime::new().unwrap();
        let resource = ResourceDef::new("/{key}/{value}/");

        let mut req = TestRequest::with_uri("/name/user1/?id=test").to_from();
        resource.match_path(req.match_info_mut());

        let res = rt
            .block_on(<(Path<(String, String)>,)>::from_request(&mut req))
            .unwrap();
        assert_eq!((res.0).0, "name");
        assert_eq!((res.0).1, "user1");

        let res = rt
            .block_on(
                <(Path<(String, String)>, Path<(String, String)>)>::from_request(
                    &mut req,
                ),
            )
            .unwrap();
        assert_eq!((res.0).0, "name");
        assert_eq!((res.0).1, "user1");
        assert_eq!((res.1).0, "name");
        assert_eq!((res.1).1, "user1");

        let () = <()>::from_request(&mut req).unwrap();
    }
}
