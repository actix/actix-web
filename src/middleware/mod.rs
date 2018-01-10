//! Middlewares
use futures::Future;

use error::Error;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

mod logger;
mod session;
mod defaultheaders;
pub use self::logger::Logger;
pub use self::defaultheaders::{DefaultHeaders, DefaultHeadersBuilder};
pub use self::session::{RequestSession, Session, SessionImpl, SessionBackend, SessionStorage,
                        CookieSessionError, CookieSessionBackend, CookieSessionBackendBuilder};

/// Middleware start result
pub enum Started {
    /// Execution completed
    Done,
    /// Moddleware error
    Err(Error),
    /// New http response got generated. If middleware generates response
    /// handler execution halts.
    Response(HttpResponse),
    /// Execution completed, runs future to completion.
    Future(Box<Future<Item=Option<HttpResponse>, Error=Error>>),
}

/// Middleware execution result
pub enum Response {
    /// Moddleware error
    Err(Error),
    /// New http response got generated
    Done(HttpResponse),
    /// Result is a future that resolves to a new http response
    Future(Box<Future<Item=HttpResponse, Error=Error>>),
}

/// Middleware finish result
pub enum Finished {
    /// Execution completed
    Done,
    /// Execution completed, but run future to completion
    Future(Box<Future<Item=(), Error=Error>>),
}

/// Middleware definition
#[allow(unused_variables)]
pub trait Middleware<S>: 'static {

    /// Method is called when request is ready. It may return
    /// future, which should resolve before next middleware get called.
    fn start(&self, req: &mut HttpRequest<S>) -> Started {
        Started::Done
    }

    /// Method is called when handler returns response,
    /// but before sending http message to peer.
    fn response(&self, req: &mut HttpRequest<S>, resp: HttpResponse) -> Response {
        Response::Done(resp)
    }

    /// Method is called after body stream get sent to peer.
    fn finish(&self, req: &mut HttpRequest<S>, resp: &HttpResponse) -> Finished {
        Finished::Done
    }
}
