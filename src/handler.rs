use std::marker::PhantomData;
use std::mem;
use std::ops::Deref;

use futures::future::{err, ok, Future};
use futures::{Async, Poll};

use error::Error;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

/// Trait defines object that could be registered as route handler
#[allow(unused_variables)]
pub trait Handler<S>: 'static {
    /// The type of value that handler will return.
    type Result: Responder;

    /// Handle request
    fn handle(&mut self, req: HttpRequest<S>) -> Self::Result;
}

/// Trait implemented by types that generate responses for clients.
///
/// Types that implement this trait can be used as the return type of a handler.
pub trait Responder {
    /// The associated item which can be returned.
    type Item: Into<Reply<HttpResponse>>;

    /// The associated error which can be returned.
    type Error: Into<Error>;

    /// Convert itself to `Reply` or `Error`.
    fn respond_to(self, req: HttpRequest) -> Result<Self::Item, Self::Error>;
}

/// Trait implemented by types that can be extracted from request.
///
/// Types that implement this trait can be used with `Route::with()` method.
pub trait FromRequest<S>: Sized {
    /// Configuration for conversion process
    type Config: Default;

    /// Future that resolves to a Self
    type Result: Into<Reply<Self>>;

    /// Convert request to a Self
    fn from_request(req: &HttpRequest<S>, cfg: &Self::Config) -> Self::Result;

    /// Convert request to a Self
    ///
    /// This method uses default extractor configuration
    fn extract(req: &HttpRequest<S>) -> Self::Result {
        Self::from_request(req, &Self::Config::default())
    }
}

/// Combines two different responder types into a single type
///
/// ```rust
/// # extern crate actix_web;
/// # extern crate futures;
/// # use futures::future::Future;
/// use futures::future::result;
/// use actix_web::{Either, Error, HttpRequest, HttpResponse, AsyncResponder};
///
/// type RegisterResult = Either<HttpResponse, Box<Future<Item=HttpResponse, Error=Error>>>;
///
///
/// fn index(req: HttpRequest) -> RegisterResult {
///     if is_a_variant() { // <- choose variant A
///         Either::A(
///             HttpResponse::BadRequest().body("Bad data"))
///     } else {
///         Either::B(      // <- variant B
///             result(Ok(HttpResponse::Ok()
///                    .content_type("text/html")
///                    .body("Hello!")))
///                    .responder())
///     }
/// }
/// # fn is_a_variant() -> bool { true }
/// # fn main() {}
/// ```
#[derive(Debug)]
pub enum Either<A, B> {
    /// First branch of the type
    A(A),
    /// Second branch of the type
    B(B),
}

impl<A, B> Responder for Either<A, B>
where
    A: Responder,
    B: Responder,
{
    type Item = Reply<HttpResponse>;
    type Error = Error;

    fn respond_to(self, req: HttpRequest) -> Result<Reply<HttpResponse>, Error> {
        match self {
            Either::A(a) => match a.respond_to(req) {
                Ok(val) => Ok(val.into()),
                Err(err) => Err(err.into()),
            },
            Either::B(b) => match b.respond_to(req) {
                Ok(val) => Ok(val.into()),
                Err(err) => Err(err.into()),
            },
        }
    }
}

impl<A, B, I, E> Future for Either<A, B>
where
    A: Future<Item = I, Error = E>,
    B: Future<Item = I, Error = E>,
{
    type Item = I;
    type Error = E;

    fn poll(&mut self) -> Poll<I, E> {
        match *self {
            Either::A(ref mut fut) => fut.poll(),
            Either::B(ref mut fut) => fut.poll(),
        }
    }
}

/// Convenience trait that converts `Future` object to a `Boxed` future
///
/// For example loading json from request's body is async operation.
///
/// ```rust
/// # extern crate actix_web;
/// # extern crate futures;
/// # #[macro_use] extern crate serde_derive;
/// use futures::future::Future;
/// use actix_web::{
///     App, HttpRequest, HttpResponse, HttpMessage, Error, AsyncResponder};
///
/// #[derive(Deserialize, Debug)]
/// struct MyObj {
///     name: String,
/// }
///
/// fn index(mut req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
///     req.json()                   // <- get JsonBody future
///        .from_err()
///        .and_then(|val: MyObj| {  // <- deserialized value
///            Ok(HttpResponse::Ok().into())
///        })
///     // Construct boxed future by using `AsyncResponder::responder()` method
///     .responder()
/// }
/// # fn main() {}
/// ```
pub trait AsyncResponder<I, E>: Sized {
    fn responder(self) -> Box<Future<Item = I, Error = E>>;
}

impl<F, I, E> AsyncResponder<I, E> for F
where
    F: Future<Item = I, Error = E> + 'static,
    I: Responder + 'static,
    E: Into<Error> + 'static,
{
    fn responder(self) -> Box<Future<Item = I, Error = E>> {
        Box::new(self)
    }
}

/// Handler<S> for Fn()
impl<F, R, S> Handler<S> for F
where
    F: Fn(HttpRequest<S>) -> R + 'static,
    R: Responder + 'static,
{
    type Result = R;

    fn handle(&mut self, req: HttpRequest<S>) -> R {
        (self)(req)
    }
}

/// Represents reply process.
///
/// Reply could be in tree different forms.
/// * Message(T) - ready item
/// * Error(Error) - error happen during reply process
/// * Future<T, Error> - reply process completes in the future
pub struct Reply<T>(ReplyItem<T>);

impl<T> Future for Reply<T> {
    type Item = T;
    type Error = Error;

    fn poll(&mut self) -> Poll<T, Error> {
        let item = mem::replace(&mut self.0, ReplyItem::None);

        match item {
            ReplyItem::Error(err) => Err(err),
            ReplyItem::Message(msg) => Ok(Async::Ready(msg)),
            ReplyItem::Future(mut fut) => match fut.poll() {
                Ok(Async::NotReady) => {
                    self.0 = ReplyItem::Future(fut);
                    Ok(Async::NotReady)
                }
                Ok(Async::Ready(msg)) => Ok(Async::Ready(msg)),
                Err(err) => Err(err),
            },
            ReplyItem::None => panic!("use after resolve"),
        }
    }
}

pub(crate) enum ReplyItem<T> {
    None,
    Error(Error),
    Message(T),
    Future(Box<Future<Item = T, Error = Error>>),
}

impl<T> Reply<T> {
    /// Create async response
    #[inline]
    pub fn async<F>(fut: F) -> Reply<T>
    where
        F: Future<Item = T, Error = Error> + 'static,
    {
        Reply(ReplyItem::Future(Box::new(fut)))
    }

    /// Send response
    #[inline]
    pub fn response<R: Into<T>>(response: R) -> Reply<T> {
        Reply(ReplyItem::Message(response.into()))
    }

    /// Send error
    #[inline]
    pub fn error<R: Into<Error>>(err: R) -> Reply<T> {
        Reply(ReplyItem::Error(err.into()))
    }

    #[inline]
    pub(crate) fn into(self) -> ReplyItem<T> {
        self.0
    }

    #[cfg(test)]
    pub(crate) fn as_msg(&self) -> &T {
        match self.0 {
            ReplyItem::Message(ref resp) => resp,
            _ => panic!(),
        }
    }

    #[cfg(test)]
    pub(crate) fn as_err(&self) -> Option<&Error> {
        match self.0 {
            ReplyItem::Error(ref err) => Some(err),
            _ => None,
        }
    }
}

impl Responder for Reply<HttpResponse> {
    type Item = Reply<HttpResponse>;
    type Error = Error;

    fn respond_to(self, _: HttpRequest) -> Result<Reply<HttpResponse>, Error> {
        Ok(self)
    }
}

impl Responder for HttpResponse {
    type Item = Reply<HttpResponse>;
    type Error = Error;

    #[inline]
    fn respond_to(self, _: HttpRequest) -> Result<Reply<HttpResponse>, Error> {
        Ok(Reply(ReplyItem::Message(self)))
    }
}

impl<T> From<T> for Reply<T> {
    #[inline]
    fn from(resp: T) -> Reply<T> {
        Reply(ReplyItem::Message(resp))
    }
}

impl<T: Responder, E: Into<Error>> Responder for Result<T, E> {
    type Item = <T as Responder>::Item;
    type Error = Error;

    fn respond_to(self, req: HttpRequest) -> Result<Self::Item, Self::Error> {
        match self {
            Ok(val) => match val.respond_to(req) {
                Ok(val) => Ok(val),
                Err(err) => Err(err.into()),
            },
            Err(err) => Err(err.into()),
        }
    }
}

impl<T, E: Into<Error>> From<Result<Reply<T>, E>> for Reply<T> {
    #[inline]
    fn from(res: Result<Reply<T>, E>) -> Self {
        match res {
            Ok(val) => val,
            Err(err) => Reply(ReplyItem::Error(err.into())),
        }
    }
}

impl<T, E: Into<Error>> From<Result<T, E>> for Reply<T> {
    #[inline]
    fn from(res: Result<T, E>) -> Self {
        match res {
            Ok(val) => Reply(ReplyItem::Message(val)),
            Err(err) => Reply(ReplyItem::Error(err.into())),
        }
    }
}

impl<T, E: Into<Error>> From<Result<Box<Future<Item = T, Error = Error>>, E>>
    for Reply<T>
{
    #[inline]
    fn from(res: Result<Box<Future<Item = T, Error = Error>>, E>) -> Self {
        match res {
            Ok(fut) => Reply(ReplyItem::Future(fut)),
            Err(err) => Reply(ReplyItem::Error(err.into())),
        }
    }
}

impl<T> From<Box<Future<Item = T, Error = Error>>> for Reply<T> {
    #[inline]
    fn from(fut: Box<Future<Item = T, Error = Error>>) -> Reply<T> {
        Reply(ReplyItem::Future(fut))
    }
}

/// Convenience type alias
pub type FutureResponse<I, E = Error> = Box<Future<Item = I, Error = E>>;

impl<I, E> Responder for Box<Future<Item = I, Error = E>>
where
    I: Responder + 'static,
    E: Into<Error> + 'static,
{
    type Item = Reply<HttpResponse>;
    type Error = Error;

    #[inline]
    fn respond_to(self, req: HttpRequest) -> Result<Reply<HttpResponse>, Error> {
        let fut = self.map_err(|e| e.into())
            .then(move |r| match r.respond_to(req) {
                Ok(reply) => match reply.into().0 {
                    ReplyItem::Message(resp) => ok(resp),
                    _ => panic!("Nested async replies are not supported"),
                },
                Err(e) => err(e),
            });
        Ok(Reply::async(fut))
    }
}

/// Trait defines object that could be registered as resource route
pub(crate) trait RouteHandler<S>: 'static {
    fn handle(&mut self, req: HttpRequest<S>) -> Reply<HttpResponse>;
}

/// Route handler wrapper for Handler
pub(crate) struct WrapHandler<S, H, R>
where
    H: Handler<S, Result = R>,
    R: Responder,
    S: 'static,
{
    h: H,
    s: PhantomData<S>,
}

impl<S, H, R> WrapHandler<S, H, R>
where
    H: Handler<S, Result = R>,
    R: Responder,
    S: 'static,
{
    pub fn new(h: H) -> Self {
        WrapHandler {
            h,
            s: PhantomData,
        }
    }
}

impl<S, H, R> RouteHandler<S> for WrapHandler<S, H, R>
where
    H: Handler<S, Result = R>,
    R: Responder + 'static,
    S: 'static,
{
    fn handle(&mut self, req: HttpRequest<S>) -> Reply<HttpResponse> {
        let req2 = req.drop_state();
        match self.h.handle(req).respond_to(req2) {
            Ok(reply) => reply.into(),
            Err(err) => Reply::response(err.into()),
        }
    }
}

/// Async route handler
pub(crate) struct AsyncHandler<S, H, F, R, E>
where
    H: Fn(HttpRequest<S>) -> F + 'static,
    F: Future<Item = R, Error = E> + 'static,
    R: Responder + 'static,
    E: Into<Error> + 'static,
    S: 'static,
{
    h: Box<H>,
    s: PhantomData<S>,
}

impl<S, H, F, R, E> AsyncHandler<S, H, F, R, E>
where
    H: Fn(HttpRequest<S>) -> F + 'static,
    F: Future<Item = R, Error = E> + 'static,
    R: Responder + 'static,
    E: Into<Error> + 'static,
    S: 'static,
{
    pub fn new(h: H) -> Self {
        AsyncHandler {
            h: Box::new(h),
            s: PhantomData,
        }
    }
}

impl<S, H, F, R, E> RouteHandler<S> for AsyncHandler<S, H, F, R, E>
where
    H: Fn(HttpRequest<S>) -> F + 'static,
    F: Future<Item = R, Error = E> + 'static,
    R: Responder + 'static,
    E: Into<Error> + 'static,
    S: 'static,
{
    fn handle(&mut self, req: HttpRequest<S>) -> Reply<HttpResponse> {
        let req2 = req.drop_state();
        let fut = (self.h)(req).map_err(|e| e.into()).then(move |r| {
            match r.respond_to(req2) {
                Ok(reply) => match reply.into().0 {
                    ReplyItem::Message(resp) => ok(resp),
                    _ => panic!("Nested async replies are not supported"),
                },
                Err(e) => err(e),
            }
        });
        Reply::async(fut)
    }
}

/// Access an application state
///
/// `S` - application state type
///
/// ## Example
///
/// ```rust
/// # extern crate bytes;
/// # extern crate actix_web;
/// # extern crate futures;
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{App, Path, State, http};
///
/// /// Application state
/// struct MyApp {msg: &'static str}
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// /// extract path info using serde
/// fn index(state: State<MyApp>, info: Path<Info>) -> String {
///     format!("{} {}!", state.msg, info.username)
/// }
///
/// fn main() {
///     let app = App::with_state(MyApp{msg: "Welcome"}).resource(
///        "/{username}/index.html",                      // <- define path parameters
///        |r| r.method(http::Method::GET).with2(index)); // <- use `with` extractor
/// }
/// ```
pub struct State<S>(HttpRequest<S>);

impl<S> Deref for State<S> {
    type Target = S;

    fn deref(&self) -> &S {
        self.0.state()
    }
}

impl<S> FromRequest<S> for State<S> {
    type Config = ();
    type Result = State<S>;

    #[inline]
    fn from_request(req: &HttpRequest<S>, _: &Self::Config) -> Self::Result {
        State(req.clone())
    }
}
