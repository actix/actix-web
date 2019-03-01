use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use std::{fmt, str};

use bytes::Bytes;
use encoding::all::UTF_8;
use encoding::types::{DecoderTrap, Encoding};
use futures::{future, Async, Future, Poll};
use mime::Mime;
use serde::de::{self, DeserializeOwned};
use serde_urlencoded;

use de::PathDeserializer;
use error::{Error, ErrorBadRequest, ErrorNotFound, UrlencodedError, ErrorConflict};
use handler::{AsyncResult, FromRequest};
use httpmessage::{HttpMessage, MessageBody, UrlEncoded};
use httprequest::HttpRequest;
use Either;

#[derive(PartialEq, Eq, PartialOrd, Ord)]
/// Extract typed information from the request's path. Information from the path is
/// URL decoded. Decoding of special characters can be disabled through `PathConfig`.
///
/// ## Example
///
/// ```rust
/// # extern crate bytes;
/// # extern crate actix_web;
/// # extern crate futures;
/// use actix_web::{http, App, Path, Result};
///
/// /// extract path info from "/{username}/{count}/index.html" url
/// /// {username} - deserializes to a String
/// /// {count} -  - deserializes to a u32
/// fn index(info: Path<(String, u32)>) -> Result<String> {
///     Ok(format!("Welcome {}! {}", info.0, info.1))
/// }
///
/// fn main() {
///     let app = App::new().resource(
///         "/{username}/{count}/index.html", // <- define path parameters
///         |r| r.method(http::Method::GET).with(index),
///     ); // <- use `with` extractor
/// }
/// ```
///
/// It is possible to extract path information to a specific type that
/// implements `Deserialize` trait from *serde*.
///
/// ```rust
/// # extern crate bytes;
/// # extern crate actix_web;
/// # extern crate futures;
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{http, App, Path, Result};
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// /// extract path info using serde
/// fn index(info: Path<Info>) -> Result<String> {
///     Ok(format!("Welcome {}!", info.username))
/// }
///
/// fn main() {
///     let app = App::new().resource(
///         "/{username}/index.html", // <- define path parameters
///         |r| r.method(http::Method::GET).with(index),
///     ); // <- use `with` extractor
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
}

impl<T> From<T> for Path<T> {
    fn from(inner: T) -> Path<T> {
        Path { inner }
    }
}

impl<T, S> FromRequest<S> for Path<T>
where
    T: DeserializeOwned,
{
    type Config = PathConfig<S>;
    type Result = Result<Self, Error>;

    #[inline]
    fn from_request(req: &HttpRequest<S>, cfg: &Self::Config) -> Self::Result {
        let req = req.clone();
        let req2 = req.clone();
        let err = Rc::clone(&cfg.ehandler);
        de::Deserialize::deserialize(PathDeserializer::new(&req, cfg.decode))
            .map_err(move |e| (*err)(e, &req2))
            .map(|inner| Path { inner })
    }
}

/// Path extractor configuration
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::{error, http, App, HttpResponse, Path, Result};
///
/// /// deserialize `Info` from request's body, max payload size is 4kb
/// fn index(info: Path<(u32, String)>) -> Result<String> {
///     Ok(format!("Welcome {}!", info.1))
/// }
///
/// fn main() {
///     let app = App::new().resource("/index.html/{id}/{name}", |r| {
///         r.method(http::Method::GET).with_config(index, |cfg| {
///             cfg.0.error_handler(|err, req| {
///                 // <- create custom error response
///                 error::InternalError::from_response(err, HttpResponse::Conflict().finish()).into()
///             });
///         })
///     });
/// }
/// ```
pub struct PathConfig<S> {
    ehandler: Rc<Fn(serde_urlencoded::de::Error, &HttpRequest<S>) -> Error>,
    decode: bool,
}
impl<S> PathConfig<S> {
    /// Set custom error handler
    pub fn error_handler<F>(&mut self, f: F) -> &mut Self
    where
        F: Fn(serde_urlencoded::de::Error, &HttpRequest<S>) -> Error + 'static,
    {
        self.ehandler = Rc::new(f);
        self
    }

    /// Disable decoding of URL encoded special charaters from the path
    pub fn disable_decoding(&mut self) -> &mut Self
    {
        self.decode = false;
        self
    }
}

impl<S> Default for PathConfig<S> {
    fn default() -> Self {
        PathConfig {
            ehandler: Rc::new(|e, _| ErrorNotFound(e)),
            decode: true,
        }
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
/// Extract typed information from the request's query.
///
/// ## Example
///
/// ```rust
/// # extern crate bytes;
/// # extern crate actix_web;
/// # extern crate futures;
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{App, Query, http};
///
///
///#[derive(Debug, Deserialize)]
///pub enum ResponseType {
///    Token,
///    Code
///}
///
///#[derive(Deserialize)]
///pub struct AuthRequest {
///    id: u64,
///    response_type: ResponseType,
///}
///
/// // use `with` extractor for query info
/// // this handler get called only if request's query contains `username` field
/// // The correct request for this handler would be `/index.html?id=64&response_type=Code"`
/// fn index(info: Query<AuthRequest>) -> String {
///     format!("Authorization request for client with id={} and type={:?}!", info.id, info.response_type)
/// }
///
/// fn main() {
///     let app = App::new().resource(
///        "/index.html",
///        |r| r.method(http::Method::GET).with(index)); // <- use `with` extractor
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

impl<T, S> FromRequest<S> for Query<T>
where
    T: de::DeserializeOwned,
{
    type Config = QueryConfig<S>;
    type Result = Result<Self, Error>;

    #[inline]
    fn from_request(req: &HttpRequest<S>, cfg: &Self::Config) -> Self::Result {
        let req2 = req.clone();
        let err = Rc::clone(&cfg.ehandler);
        serde_urlencoded::from_str::<T>(req.query_string())
            .map_err(move |e| (*err)(e, &req2))
            .map(Query)
    }
}

/// Query extractor configuration
///
/// ```rust
/// # extern crate actix_web;
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{error, http, App, HttpResponse, Query, Result};
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// /// deserialize `Info` from request's body, max payload size is 4kb
/// fn index(info: Query<Info>) -> Result<String> {
///     Ok(format!("Welcome {}!", info.username))
/// }
///
/// fn main() {
///     let app = App::new().resource("/index.html", |r| {
///         r.method(http::Method::GET).with_config(index, |cfg| {
///             cfg.0.error_handler(|err, req| {
///                 // <- create custom error response
///                 error::InternalError::from_response(err, HttpResponse::Conflict().finish()).into()
///             });
///         })
///     });
/// }
/// ```
pub struct QueryConfig<S> {
    ehandler: Rc<Fn(serde_urlencoded::de::Error, &HttpRequest<S>) -> Error>,
}
impl<S> QueryConfig<S> {
    /// Set custom error handler
    pub fn error_handler<F>(&mut self, f: F) -> &mut Self
    where
        F: Fn(serde_urlencoded::de::Error, &HttpRequest<S>) -> Error + 'static,
    {
        self.ehandler = Rc::new(f);
        self
    }
}

impl<S> Default for QueryConfig<S> {
    fn default() -> Self {
        QueryConfig {
            ehandler: Rc::new(|e, _| e.into()),
        }
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
/// [**FormConfig**](dev/struct.FormConfig.html) allows to configure extraction
/// process.
///
/// ## Example
///
/// ```rust
/// # extern crate actix_web;
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{App, Form, Result};
///
/// #[derive(Deserialize)]
/// struct FormData {
///     username: String,
/// }
///
/// /// extract form data using serde
/// /// this handler get called only if content type is *x-www-form-urlencoded*
/// /// and content of the request could be deserialized to a `FormData` struct
/// fn index(form: Form<FormData>) -> Result<String> {
///     Ok(format!("Welcome {}!", form.username))
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

impl<T, S> FromRequest<S> for Form<T>
where
    T: DeserializeOwned + 'static,
    S: 'static,
{
    type Config = FormConfig<S>;
    type Result = Box<Future<Item = Self, Error = Error>>;

    #[inline]
    fn from_request(req: &HttpRequest<S>, cfg: &Self::Config) -> Self::Result {
        let req2 = req.clone();
        let err = Rc::clone(&cfg.ehandler);
        Box::new(
            UrlEncoded::new(req)
                .limit(cfg.limit)
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
/// # extern crate actix_web;
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{http, App, Form, Result};
///
/// #[derive(Deserialize)]
/// struct FormData {
///     username: String,
/// }
///
/// /// extract form data using serde.
/// /// custom configuration is used for this handler, max payload size is 4k
/// fn index(form: Form<FormData>) -> Result<String> {
///     Ok(format!("Welcome {}!", form.username))
/// }
///
/// fn main() {
///     let app = App::new().resource(
///         "/index.html",
///         |r| {
///             r.method(http::Method::GET)
///                 // register form handler and change form extractor configuration
///                 .with_config(index, |cfg| {cfg.0.limit(4096);})
///         },
///     );
/// }
/// ```
pub struct FormConfig<S> {
    limit: usize,
    ehandler: Rc<Fn(UrlencodedError, &HttpRequest<S>) -> Error>,
}

impl<S> FormConfig<S> {
    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(&mut self, limit: usize) -> &mut Self {
        self.limit = limit;
        self
    }

    /// Set custom error handler
    pub fn error_handler<F>(&mut self, f: F) -> &mut Self
    where
        F: Fn(UrlencodedError, &HttpRequest<S>) -> Error + 'static,
    {
        self.ehandler = Rc::new(f);
        self
    }
}

impl<S> Default for FormConfig<S> {
    fn default() -> Self {
        FormConfig {
            limit: 262_144,
            ehandler: Rc::new(|e, _| e.into()),
        }
    }
}

/// Request payload extractor.
///
/// Loads request's payload and construct Bytes instance.
///
/// [**PayloadConfig**](dev/struct.PayloadConfig.html) allows to configure
/// extraction process.
///
/// ## Example
///
/// ```rust
/// extern crate bytes;
/// # extern crate actix_web;
/// use actix_web::{http, App, Result};
///
/// /// extract text data from request
/// fn index(body: bytes::Bytes) -> Result<String> {
///     Ok(format!("Body {:?}!", body))
/// }
///
/// fn main() {
///     let app = App::new()
///         .resource("/index.html", |r| r.method(http::Method::GET).with(index));
/// }
/// ```
impl<S: 'static> FromRequest<S> for Bytes {
    type Config = PayloadConfig;
    type Result = Result<Box<Future<Item = Self, Error = Error>>, Error>;

    #[inline]
    fn from_request(req: &HttpRequest<S>, cfg: &Self::Config) -> Self::Result {
        // check content-type
        cfg.check_mimetype(req)?;

        Ok(Box::new(MessageBody::new(req).limit(cfg.limit).from_err()))
    }
}

/// Extract text information from the request's body.
///
/// Text extractor automatically decode body according to the request's charset.
///
/// [**PayloadConfig**](dev/struct.PayloadConfig.html) allows to configure
/// extraction process.
///
/// ## Example
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::{http, App, Result};
///
/// /// extract text data from request
/// fn index(body: String) -> Result<String> {
///     Ok(format!("Body {}!", body))
/// }
///
/// fn main() {
///     let app = App::new().resource("/index.html", |r| {
///         r.method(http::Method::GET)
///                .with_config(index, |cfg| { // <- register handler with extractor params
///                   cfg.0.limit(4096);  // <- limit size of the payload
///                 })
///     });
/// }
/// ```
impl<S: 'static> FromRequest<S> for String {
    type Config = PayloadConfig;
    type Result = Result<Box<Future<Item = String, Error = Error>>, Error>;

    #[inline]
    fn from_request(req: &HttpRequest<S>, cfg: &Self::Config) -> Self::Result {
        // check content-type
        cfg.check_mimetype(req)?;

        // check charset
        let encoding = req.encoding()?;

        Ok(Box::new(
            MessageBody::new(req)
                .limit(cfg.limit)
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
/// # extern crate actix_web;
/// extern crate rand;
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{http, App, Result, HttpRequest, Error, FromRequest};
/// use actix_web::error::ErrorBadRequest;
///
/// #[derive(Debug, Deserialize)]
/// struct Thing { name: String }
///
/// impl<S> FromRequest<S> for Thing {
///     type Config = ();
///     type Result = Result<Thing, Error>;
///
///     #[inline]
///     fn from_request(req: &HttpRequest<S>, _cfg: &Self::Config) -> Self::Result {
///         if rand::random() {
///             Ok(Thing { name: "thingy".into() })
///         } else {
///             Err(ErrorBadRequest("no luck"))
///         }
///
///     }
/// }
///
/// /// extract text data from request
/// fn index(supplied_thing: Option<Thing>) -> Result<String> {
///     match supplied_thing {
///         // Puns not intended
///         Some(thing) => Ok(format!("Got something: {:?}", thing)),
///         None => Ok(format!("No thing!"))
///     }
/// }
///
/// fn main() {
///     let app = App::new().resource("/users/:first", |r| {
///         r.method(http::Method::POST).with(index)
///     });
/// }
/// ```
impl<T: 'static, S: 'static> FromRequest<S> for Option<T>
where
    T: FromRequest<S>,
{
    type Config = T::Config;
    type Result = Box<Future<Item = Option<T>, Error = Error>>;

    #[inline]
    fn from_request(req: &HttpRequest<S>, cfg: &Self::Config) -> Self::Result {
        Box::new(T::from_request(req, cfg).into().then(|r| match r {
            Ok(v) => future::ok(Some(v)),
            Err(_) => future::ok(None),
        }))
    }
}

/// Extract either one of two fields from the request.
///
/// If both or none of the fields can be extracted, the default behaviour is to prefer the first
/// successful, last that failed. The behaviour can be changed by setting the appropriate
/// ```EitherCollisionStrategy```.
///
/// CAVEAT: Most of the time both extractors will be run. Make sure that the extractors you specify
/// can be run one after another (or in parallel). This will always fail for extractors that modify
/// the request state (such as the `Form` extractors that read in the body stream).
/// So Either<Form<A>, Form<B>> will not work correctly - it will only succeed if it matches the first
/// option, but will always fail to match the second (since the body stream will be at the end, and
/// appear to be empty).
///
/// ## Example
///
/// ```rust
/// # extern crate actix_web;
/// extern crate rand;
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{http, App, Result, HttpRequest, Error, FromRequest};
/// use actix_web::error::ErrorBadRequest;
/// use actix_web::Either;
///
/// #[derive(Debug, Deserialize)]
/// struct Thing { name: String }
///
/// #[derive(Debug, Deserialize)]
/// struct OtherThing { id: String }
///
/// impl<S> FromRequest<S> for Thing {
///     type Config = ();
///     type Result = Result<Thing, Error>;
///
///     #[inline]
///     fn from_request(req: &HttpRequest<S>, _cfg: &Self::Config) -> Self::Result {
///         if rand::random() {
///             Ok(Thing { name: "thingy".into() })
///         } else {
///             Err(ErrorBadRequest("no luck"))
///         }
///     }
/// }
///
/// impl<S> FromRequest<S> for OtherThing {
///     type Config = ();
///     type Result = Result<OtherThing, Error>;
///
///     #[inline]
///     fn from_request(req: &HttpRequest<S>, _cfg: &Self::Config) -> Self::Result {
///         if rand::random() {
///             Ok(OtherThing { id: "otherthingy".into() })
///         } else {
///             Err(ErrorBadRequest("no luck"))
///         }
///     }
/// }
///
/// /// extract text data from request
/// fn index(supplied_thing: Either<Thing, OtherThing>) -> Result<String> {
///     match supplied_thing {
///         Either::A(thing) => Ok(format!("Got something: {:?}", thing)),
///         Either::B(other_thing) => Ok(format!("Got anotherthing: {:?}", other_thing))
///     }
/// }
///
/// fn main() {
///     let app = App::new().resource("/users/:first", |r| {
///         r.method(http::Method::POST).with(index)
///     });
/// }
/// ```
impl<A: 'static, B: 'static, S: 'static> FromRequest<S> for Either<A,B> where A: FromRequest<S>, B: FromRequest<S> {
    type Config = EitherConfig<A,B,S>;
    type Result = AsyncResult<Either<A,B>>;

    #[inline]
    fn from_request(req: &HttpRequest<S>, cfg: &Self::Config) -> Self::Result {
        let a = A::from_request(&req.clone(), &cfg.a).into().map(|a| Either::A(a));
        let b = B::from_request(req, &cfg.b).into().map(|b| Either::B(b));

        match &cfg.collision_strategy {
            EitherCollisionStrategy::PreferA => AsyncResult::future(Box::new(a.or_else(|_| b))),
            EitherCollisionStrategy::PreferB => AsyncResult::future(Box::new(b.or_else(|_| a))),
            EitherCollisionStrategy::FastestSuccessful => AsyncResult::future(Box::new(a.select2(b).then( |r| match r {
                Ok(future::Either::A((ares, _b))) => AsyncResult::ok(ares),
                Ok(future::Either::B((bres, _a))) => AsyncResult::ok(bres),
                Err(future::Either::A((_aerr, b))) => AsyncResult::future(Box::new(b)),
                Err(future::Either::B((_berr, a))) => AsyncResult::future(Box::new(a))
            }))),
            EitherCollisionStrategy::ErrorA => AsyncResult::future(Box::new(b.then(|r| match r {
                Err(_berr) => AsyncResult::future(Box::new(a)),
                Ok(b) => AsyncResult::future(Box::new(a.then( |r| match r {
                    Ok(_a) => Err(ErrorConflict("Both wings of either extractor completed")),
                    Err(_arr) => Ok(b)
                })))
            }))),
            EitherCollisionStrategy::ErrorB => AsyncResult::future(Box::new(a.then(|r| match r {
                Err(_aerr) => AsyncResult::future(Box::new(b)),
                Ok(a) => AsyncResult::future(Box::new(b.then( |r| match r {
                    Ok(_b) => Err(ErrorConflict("Both wings of either extractor completed")),
                    Err(_berr) => Ok(a)
                })))
            }))),
        }
    }
}

/// Defines the result if neither or both of the extractors supplied to an Either<A,B> extractor succeed.
#[derive(Debug)]
pub enum EitherCollisionStrategy {
    /// If both are successful, return A, if both fail, return error of B
    PreferA,
    /// If both are successful, return B, if both fail, return error of A
    PreferB,
    /// Return result of the faster, error of the slower if both fail
    FastestSuccessful,
    /// Return error if both succeed, return error of A if both fail
    ErrorA,
    /// Return error if both succeed, return error of B if both fail
    ErrorB
}

impl Default for EitherCollisionStrategy {
    fn default() -> Self {
        EitherCollisionStrategy::FastestSuccessful
    }
}

///Determines Either extractor configuration
///
///By default `EitherCollisionStrategy::FastestSuccessful` is used.
pub struct EitherConfig<A,B,S> where A: FromRequest<S>, B: FromRequest<S> {
    a: A::Config,
    b: B::Config,
    collision_strategy: EitherCollisionStrategy
}

impl<A,B,S> Default for EitherConfig<A,B,S> where A: FromRequest<S>, B: FromRequest<S> {
    fn default() -> Self {
        EitherConfig {
            a: A::Config::default(),
            b: B::Config::default(),
            collision_strategy: EitherCollisionStrategy::default()
        }
    }
}

/// Optionally extract a field from the request or extract the Error if unsuccessful
///
/// If the FromRequest for T fails, inject Err into handler rather than returning an error response
///
/// ## Example
///
/// ```rust
/// # extern crate actix_web;
/// extern crate rand;
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{http, App, Result, HttpRequest, Error, FromRequest};
/// use actix_web::error::ErrorBadRequest;
///
/// #[derive(Debug, Deserialize)]
/// struct Thing { name: String }
///
/// impl<S> FromRequest<S> for Thing {
///     type Config = ();
///     type Result = Result<Thing, Error>;
///
///     #[inline]
///     fn from_request(req: &HttpRequest<S>, _cfg: &Self::Config) -> Self::Result {
///         if rand::random() {
///             Ok(Thing { name: "thingy".into() })
///         } else {
///             Err(ErrorBadRequest("no luck"))
///         }
///
///     }
/// }
///
/// /// extract text data from request
/// fn index(supplied_thing: Result<Thing>) -> Result<String> {
///     match supplied_thing {
///         Ok(thing) => Ok(format!("Got thing: {:?}", thing)),
///         Err(e) => Ok(format!("Error extracting thing: {}", e))
///     }
/// }
///
/// fn main() {
///     let app = App::new().resource("/users/:first", |r| {
///         r.method(http::Method::POST).with(index)
///     });
/// }
/// ```
impl<T: 'static, S: 'static> FromRequest<S> for Result<T, Error>
where
    T: FromRequest<S>,
{
    type Config = T::Config;
    type Result = Box<Future<Item = Result<T, Error>, Error = Error>>;

    #[inline]
    fn from_request(req: &HttpRequest<S>, cfg: &Self::Config) -> Self::Result {
        Box::new(T::from_request(req, cfg).into().then(future::ok))
    }
}

/// Payload configuration for request's payload.
pub struct PayloadConfig {
    limit: usize,
    mimetype: Option<Mime>,
}

impl PayloadConfig {
    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(&mut self, limit: usize) -> &mut Self {
        self.limit = limit;
        self
    }

    /// Set required mime-type of the request. By default mime type is not
    /// enforced.
    pub fn mimetype(&mut self, mt: Mime) -> &mut Self {
        self.mimetype = Some(mt);
        self
    }

    fn check_mimetype<S>(&self, req: &HttpRequest<S>) -> Result<(), Error> {
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

impl Default for PayloadConfig {
    fn default() -> Self {
        PayloadConfig {
            limit: 262_144,
            mimetype: None,
        }
    }
}

macro_rules! tuple_from_req ({$fut_type:ident, $(($n:tt, $T:ident)),+} => {

    /// FromRequest implementation for tuple
    impl<S, $($T: FromRequest<S> + 'static),+> FromRequest<S> for ($($T,)+)
    where
        S: 'static,
    {
        type Config = ($($T::Config,)+);
        type Result = Box<Future<Item = ($($T,)+), Error = Error>>;

        fn from_request(req: &HttpRequest<S>, cfg: &Self::Config) -> Self::Result {
            Box::new($fut_type {
                s: PhantomData,
                items: <($(Option<$T>,)+)>::default(),
                futs: ($(Some($T::from_request(req, &cfg.$n).into()),)+),
            })
        }
    }

    struct $fut_type<S, $($T: FromRequest<S>),+>
    where
        S: 'static,
    {
        s: PhantomData<S>,
        items: ($(Option<$T>,)+),
        futs: ($(Option<AsyncResult<$T>>,)+),
    }

    impl<S, $($T: FromRequest<S>),+> Future for $fut_type<S, $($T),+>
    where
        S: 'static,
    {
        type Item = ($($T,)+);
        type Error = Error;

        fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
            let mut ready = true;

            $(
                if self.futs.$n.is_some() {
                    match self.futs.$n.as_mut().unwrap().poll() {
                        Ok(Async::Ready(item)) => {
                            self.items.$n = Some(item);
                            self.futs.$n.take();
                        }
                        Ok(Async::NotReady) => ready = false,
                        Err(e) => return Err(e),
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

impl<S> FromRequest<S> for () {
    type Config = ();
    type Result = Self;
    fn from_request(_req: &HttpRequest<S>, _cfg: &Self::Config) -> Self::Result {}
}

tuple_from_req!(TupleFromRequest1, (0, A));
tuple_from_req!(TupleFromRequest2, (0, A), (1, B));
tuple_from_req!(TupleFromRequest3, (0, A), (1, B), (2, C));
tuple_from_req!(TupleFromRequest4, (0, A), (1, B), (2, C), (3, D));
tuple_from_req!(TupleFromRequest5, (0, A), (1, B), (2, C), (3, D), (4, E));
tuple_from_req!(
    TupleFromRequest6,
    (0, A),
    (1, B),
    (2, C),
    (3, D),
    (4, E),
    (5, F)
);
tuple_from_req!(
    TupleFromRequest7,
    (0, A),
    (1, B),
    (2, C),
    (3, D),
    (4, E),
    (5, F),
    (6, G)
);
tuple_from_req!(
    TupleFromRequest8,
    (0, A),
    (1, B),
    (2, C),
    (3, D),
    (4, E),
    (5, F),
    (6, G),
    (7, H)
);
tuple_from_req!(
    TupleFromRequest9,
    (0, A),
    (1, B),
    (2, C),
    (3, D),
    (4, E),
    (5, F),
    (6, G),
    (7, H),
    (8, I)
);

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use futures::{Async, Future};
    use http::header;
    use mime;
    use resource::Resource;
    use router::{ResourceDef, Router};
    use test::TestRequest;

    #[derive(Deserialize, Debug, PartialEq)]
    struct Info {
        hello: String,
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct OtherInfo {
        bye: String,
    }

    #[test]
    fn test_bytes() {
        let cfg = PayloadConfig::default();
        let req = TestRequest::with_header(header::CONTENT_LENGTH, "11")
            .set_payload(Bytes::from_static(b"hello=world"))
            .finish();

        match Bytes::from_request(&req, &cfg).unwrap().poll().unwrap() {
            Async::Ready(s) => {
                assert_eq!(s, Bytes::from_static(b"hello=world"));
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_string() {
        let cfg = PayloadConfig::default();
        let req = TestRequest::with_header(header::CONTENT_LENGTH, "11")
            .set_payload(Bytes::from_static(b"hello=world"))
            .finish();

        match String::from_request(&req, &cfg).unwrap().poll().unwrap() {
            Async::Ready(s) => {
                assert_eq!(s, "hello=world");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_form() {
        let req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        ).header(header::CONTENT_LENGTH, "11")
        .set_payload(Bytes::from_static(b"hello=world"))
        .finish();

        let mut cfg = FormConfig::default();
        cfg.limit(4096);
        match Form::<Info>::from_request(&req, &cfg).poll().unwrap() {
            Async::Ready(s) => {
                assert_eq!(s.hello, "world");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_option() {
        let req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        ).finish();

        let mut cfg = FormConfig::default();
        cfg.limit(4096);

        match Option::<Form<Info>>::from_request(&req, &cfg)
            .poll()
            .unwrap()
        {
            Async::Ready(r) => assert_eq!(r, None),
            _ => unreachable!(),
        }

        let req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        ).header(header::CONTENT_LENGTH, "9")
        .set_payload(Bytes::from_static(b"hello=world"))
        .finish();

        match Option::<Form<Info>>::from_request(&req, &cfg)
            .poll()
            .unwrap()
        {
            Async::Ready(r) => assert_eq!(
                r,
                Some(Form(Info {
                    hello: "world".into()
                }))
            ),
            _ => unreachable!(),
        }

        let req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        ).header(header::CONTENT_LENGTH, "9")
        .set_payload(Bytes::from_static(b"bye=world"))
        .finish();

        match Option::<Form<Info>>::from_request(&req, &cfg)
            .poll()
            .unwrap()
        {
            Async::Ready(r) => assert_eq!(r, None),
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_either() {
        let req = TestRequest::default().finish();
        let mut cfg: EitherConfig<Query<Info>, Query<OtherInfo>, _> = EitherConfig::default();

        assert!(Either::<Query<Info>, Query<OtherInfo>>::from_request(&req, &cfg).poll().is_err());

        let req = TestRequest::default().uri("/index?hello=world").finish();

        match Either::<Query<Info>, Query<OtherInfo>>::from_request(&req, &cfg).poll().unwrap() {
            Async::Ready(r) => assert_eq!(r, Either::A(Query(Info { hello: "world".into() }))),
            _ => unreachable!(),
        }

        let req = TestRequest::default().uri("/index?bye=world").finish();
        match Either::<Query<Info>, Query<OtherInfo>>::from_request(&req, &cfg).poll().unwrap() {
            Async::Ready(r) => assert_eq!(r, Either::B(Query(OtherInfo { bye: "world".into() }))),
            _ => unreachable!(),
        }

        let req = TestRequest::default().uri("/index?hello=world&bye=world").finish();
        cfg.collision_strategy = EitherCollisionStrategy::PreferA;

        match Either::<Query<Info>, Query<OtherInfo>>::from_request(&req, &cfg).poll().unwrap() {
            Async::Ready(r) => assert_eq!(r, Either::A(Query(Info { hello: "world".into() }))),
            _ => unreachable!(),
        }

        cfg.collision_strategy = EitherCollisionStrategy::PreferB;

        match Either::<Query<Info>, Query<OtherInfo>>::from_request(&req, &cfg).poll().unwrap() {
            Async::Ready(r) => assert_eq!(r, Either::B(Query(OtherInfo { bye: "world".into() }))),
            _ => unreachable!(),
        }

        cfg.collision_strategy = EitherCollisionStrategy::ErrorA;
        assert!(Either::<Query<Info>, Query<OtherInfo>>::from_request(&req, &cfg).poll().is_err());

        cfg.collision_strategy = EitherCollisionStrategy::FastestSuccessful;
        assert!(Either::<Query<Info>, Query<OtherInfo>>::from_request(&req, &cfg).poll().is_ok());
    }

    #[test]
    fn test_result() {
        let req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        ).header(header::CONTENT_LENGTH, "11")
        .set_payload(Bytes::from_static(b"hello=world"))
        .finish();

        match Result::<Form<Info>, Error>::from_request(&req, &FormConfig::default())
            .poll()
            .unwrap()
        {
            Async::Ready(Ok(r)) => assert_eq!(
                r,
                Form(Info {
                    hello: "world".into()
                })
            ),
            _ => unreachable!(),
        }

        let req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        ).header(header::CONTENT_LENGTH, "9")
        .set_payload(Bytes::from_static(b"bye=world"))
        .finish();

        match Result::<Form<Info>, Error>::from_request(&req, &FormConfig::default())
            .poll()
            .unwrap()
        {
            Async::Ready(r) => assert!(r.is_err()),
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_payload_config() {
        let req = TestRequest::default().finish();
        let mut cfg = PayloadConfig::default();
        cfg.mimetype(mime::APPLICATION_JSON);
        assert!(cfg.check_mimetype(&req).is_err());

        let req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        ).finish();
        assert!(cfg.check_mimetype(&req).is_err());

        let req =
            TestRequest::with_header(header::CONTENT_TYPE, "application/json").finish();
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
        let req = TestRequest::with_uri("/name/user1/?id=test").finish();

        let mut router = Router::<()>::default();
        router.register_resource(Resource::new(ResourceDef::new("/{key}/{value}/")));
        let info = router.recognize(&req, &(), 0);
        let req = req.with_route_info(info);

        let s = Path::<MyStruct>::from_request(&req, &PathConfig::default()).unwrap();
        assert_eq!(s.key, "name");
        assert_eq!(s.value, "user1");

        let s = Path::<(String, String)>::from_request(&req, &PathConfig::default()).unwrap();
        assert_eq!(s.0, "name");
        assert_eq!(s.1, "user1");

        let s = Query::<Id>::from_request(&req, &QueryConfig::default()).unwrap();
        assert_eq!(s.id, "test");

        let mut router = Router::<()>::default();
        router.register_resource(Resource::new(ResourceDef::new("/{key}/{value}/")));
        let req = TestRequest::with_uri("/name/32/").finish();
        let info = router.recognize(&req, &(), 0);
        let req = req.with_route_info(info);

        let s = Path::<Test2>::from_request(&req, &PathConfig::default()).unwrap();
        assert_eq!(s.as_ref().key, "name");
        assert_eq!(s.value, 32);

        let s = Path::<(String, u8)>::from_request(&req, &PathConfig::default()).unwrap();
        assert_eq!(s.0, "name");
        assert_eq!(s.1, 32);

        let res = Path::<Vec<String>>::extract(&req).unwrap();
        assert_eq!(res[0], "name".to_owned());
        assert_eq!(res[1], "32".to_owned());
    }

    #[test]
    fn test_extract_path_single() {
        let mut router = Router::<()>::default();
        router.register_resource(Resource::new(ResourceDef::new("/{value}/")));

        let req = TestRequest::with_uri("/32/").finish();
        let info = router.recognize(&req, &(), 0);
        let req = req.with_route_info(info);
        assert_eq!(*Path::<i8>::from_request(&req, &&PathConfig::default()).unwrap(), 32);
    }

    #[test]
    fn test_extract_path_decode() {
        let mut router = Router::<()>::default();
        router.register_resource(Resource::new(ResourceDef::new("/{value}/")));

        macro_rules! test_single_value {
            ($value:expr, $expected:expr) => {
                {
                    let req = TestRequest::with_uri($value).finish();
                    let info = router.recognize(&req, &(), 0);
                    let req = req.with_route_info(info);
                    assert_eq!(*Path::<String>::from_request(&req, &PathConfig::default()).unwrap(), $expected);
                }
            }
        }

        test_single_value!("/%25/", "%");
        test_single_value!("/%40%C2%A3%24%25%5E%26%2B%3D/", "@£$%^&+=");
        test_single_value!("/%2B/", "+");
        test_single_value!("/%252B/", "%2B");
        test_single_value!("/%2F/", "/");
        test_single_value!("/%252F/", "%2F");
        test_single_value!("/http%3A%2F%2Flocalhost%3A80%2Ffoo/", "http://localhost:80/foo");
        test_single_value!("/%2Fvar%2Flog%2Fsyslog/", "/var/log/syslog");
        test_single_value!(
            "/http%3A%2F%2Flocalhost%3A80%2Ffile%2F%252Fvar%252Flog%252Fsyslog/",
            "http://localhost:80/file/%2Fvar%2Flog%2Fsyslog"
        );

        let req = TestRequest::with_uri("/%25/7/?id=test").finish();

        let mut router = Router::<()>::default();
        router.register_resource(Resource::new(ResourceDef::new("/{key}/{value}/")));
        let info = router.recognize(&req, &(), 0);
        let req = req.with_route_info(info);

        let s = Path::<Test2>::from_request(&req, &PathConfig::default()).unwrap();
        assert_eq!(s.key, "%");
        assert_eq!(s.value, 7);

        let s = Path::<(String, String)>::from_request(&req, &PathConfig::default()).unwrap();
        assert_eq!(s.0, "%");
        assert_eq!(s.1, "7");
    }

    #[test]
    fn test_extract_path_no_decode() {
        let mut router = Router::<()>::default();
        router.register_resource(Resource::new(ResourceDef::new("/{value}/")));

        let req = TestRequest::with_uri("/%25/").finish();
        let info = router.recognize(&req, &(), 0);
        let req = req.with_route_info(info);
        assert_eq!(
            *Path::<String>::from_request(
                &req,
                &&PathConfig::default().disable_decoding()
            ).unwrap(),
            "%25"
        );
    }

    #[test]
    fn test_tuple_extract() {
        let mut router = Router::<()>::default();
        router.register_resource(Resource::new(ResourceDef::new("/{key}/{value}/")));

        let req = TestRequest::with_uri("/name/user1/?id=test").finish();
        let info = router.recognize(&req, &(), 0);
        let req = req.with_route_info(info);

        let res = match <(Path<(String, String)>,)>::extract(&req).poll() {
            Ok(Async::Ready(res)) => res,
            _ => panic!("error"),
        };
        assert_eq!((res.0).0, "name");
        assert_eq!((res.0).1, "user1");

        let res = match <(Path<(String, String)>, Path<(String, String)>)>::extract(&req)
            .poll()
        {
            Ok(Async::Ready(res)) => res,
            _ => panic!("error"),
        };
        assert_eq!((res.0).0, "name");
        assert_eq!((res.0).1, "user1");
        assert_eq!((res.1).0, "name");
        assert_eq!((res.1).1, "user1");

        let () = <()>::extract(&req);
    }
}
