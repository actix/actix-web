//! Middlewares
#![allow(unused_imports, dead_code)]

use std::rc::Rc;
use futures::{Async, Future, Poll};

use error::Error;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

mod logger;
pub use self::logger::Logger;

/// Middleware start result
pub enum Started {
    /// Execution completed
    Done(HttpRequest),
    /// New http response got generated. If middleware generates response
    /// handler execution halts.
    Response(HttpRequest, HttpResponse),
    /// Execution completed, runs future to completion.
    Future(Box<Future<Item=(HttpRequest, Option<HttpResponse>), Error=(HttpRequest, HttpResponse)>>),
}

/// Middleware execution result
pub enum Response {
    /// New http response got generated
    Response(HttpResponse),
    /// Result is a future that resolves to a new http response
    Future(Box<Future<Item=HttpResponse, Error=HttpResponse>>),
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
pub trait Middleware {

    /// Method is called when request is ready. It may return
    /// future, which should resolve before next middleware get called.
    fn start(&self, req: HttpRequest) -> Started {
        Started::Done(req)
    }

    /// Method is called when handler returns response,
    /// but before sending body stream to peer.
    fn response(&self, req: &mut HttpRequest, resp: HttpResponse) -> Response {
        Response::Response(resp)
    }

    /// Method is called after http response get sent to peer.
    fn finish(&self, req: &mut HttpRequest, resp: &HttpResponse) -> Finished {
        Finished::Done
    }
}
