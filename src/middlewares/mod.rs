//! Middlewares
use std::rc::Rc;
use std::error::Error;
use futures::{Async, Future, Poll};
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

mod logger;
pub use self::logger::Logger;

/// Middleware start result
pub enum Started {
    /// Execution completed
    Done,
    /// New http response got generated. If middleware generates response
    /// handler execution halts.
    Response(HttpResponse),
    /// Execution completed, but run future to completion.
    Future(Box<Future<Item=(), Error=HttpResponse>>),
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
    Future(Box<Future<Item=(), Error=Box<Error>>>),
}

/// Middleware definition
#[allow(unused_variables)]
pub trait Middleware {

    /// Method is called when request is ready. It may return
    /// future, which should resolve before next middleware get called.
    fn start(&self, req: &mut HttpRequest) -> Started {
        Started::Done
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

/// Middlewares executor
pub(crate) struct MiddlewaresExecutor {
    state: ExecutorState,
    fut: Option<Box<Future<Item=HttpResponse, Error=HttpResponse>>>,
    started: Option<Box<Future<Item=(), Error=HttpResponse>>>,
    finished: Option<Box<Future<Item=(), Error=Box<Error>>>>,
    middlewares: Option<Rc<Vec<Box<Middleware>>>>,
}

enum ExecutorState {
    None,
    Starting(usize),
    Started(usize),
    Processing(usize, usize),
    Finishing(usize),
}

impl Default for MiddlewaresExecutor {

    fn default() -> MiddlewaresExecutor {
        MiddlewaresExecutor {
            fut: None,
            started: None,
            finished: None,
            state: ExecutorState::None,
            middlewares: None,
        }
    }
}

impl MiddlewaresExecutor {

    pub fn start(&mut self, mw: Rc<Vec<Box<Middleware>>>) {
        self.state = ExecutorState::Starting(0);
        self.middlewares = Some(mw);
    }

    pub fn starting(&mut self, req: &mut HttpRequest) -> Poll<Option<HttpResponse>, ()> {
        if let Some(ref middlewares) = self.middlewares {
            let state = &mut self.state;
            if let ExecutorState::Starting(mut idx) = *state {
                loop {
                    // poll latest fut
                    if let Some(ref mut fut) = self.started {
                        match fut.poll() {
                            Ok(Async::NotReady) => return Ok(Async::NotReady),
                            Ok(Async::Ready(())) => idx += 1,
                            Err(response) => {
                                *state = ExecutorState::Started(idx);
                                return Ok(Async::Ready(Some(response)))
                            }
                        }
                    }
                    self.started = None;

                    if idx >= middlewares.len() {
                        *state = ExecutorState::Started(idx-1);
                        return Ok(Async::Ready(None))
                    } else {
                        match middlewares[idx].start(req) {
                            Started::Done => idx += 1,
                            Started::Response(resp) => {
                                *state = ExecutorState::Started(idx);
                                return Ok(Async::Ready(Some(resp)))
                            },
                            Started::Future(fut) => {
                                self.started = Some(fut);
                            },
                        }
                    }
                }
            }
        }
        Ok(Async::Ready(None))
    }

    pub fn processing(&mut self, req: &mut HttpRequest) -> Poll<Option<HttpResponse>, ()> {
        if let Some(ref middlewares) = self.middlewares {
            let state = &mut self.state;
            match *state {
                ExecutorState::Processing(mut idx, total) => {
                    loop {
                        // poll latest fut
                        let mut resp = match self.fut.as_mut().unwrap().poll() {
                            Ok(Async::NotReady) => return Ok(Async::NotReady),
                            Ok(Async::Ready(response)) | Err(response) => {
                                idx += 1;
                                response
                            }
                        };
                        self.fut = None;

                        loop {
                            if idx == 0 {
                                *state = ExecutorState::Finishing(total);
                                return Ok(Async::Ready(Some(resp)))
                            } else {
                                match middlewares[idx].response(req, resp) {
                                    Response::Response(r) => {
                                        idx -= 1;
                                        resp = r
                                    },
                                    Response::Future(fut) => {
                                        self.fut = Some(fut);
                                        break
                                    },
                                }
                            }
                        }
                    }
                }
                _ => Ok(Async::Ready(None))
            }
        } else {
            Ok(Async::Ready(None))
        }
    }

    pub fn finishing(&mut self, req: &mut HttpRequest, resp: &HttpResponse) -> Poll<(), ()> {
        if let Some(ref middlewares) = self.middlewares {
            let state = &mut self.state;
            if let ExecutorState::Finishing(mut idx) = *state {
                loop {
                    // poll latest fut
                    if let Some(ref mut fut) = self.finished {
                        match fut.poll() {
                            Ok(Async::NotReady) => return Ok(Async::NotReady),
                            Ok(Async::Ready(())) => idx -= 1,
                            Err(err) => {
                                error!("Middleware finish error: {}", err);
                            }
                        }
                    }
                    self.finished = None;

                    match middlewares[idx].finish(req, resp) {
                        Finished::Done => {
                            if idx == 0 {
                                return Ok(Async::Ready(()))
                            } else {
                                idx -= 1
                            }
                        }
                        Finished::Future(fut) => {
                            self.finished = Some(fut);
                        },
                    }
                }
            }
        }
        Ok(Async::Ready(()))
    }

    pub fn response(&mut self, req: &mut HttpRequest, resp: HttpResponse)
                    -> Option<HttpResponse>
    {
        if let Some(ref middlewares) = self.middlewares {
            let mut resp = resp;
            let state = &mut self.state;
            match *state {
                ExecutorState::Started(mut idx) => {
                    let total = idx;
                    loop {
                        resp = match middlewares[idx].response(req, resp) {
                            Response::Response(r) => {
                                if idx == 0 {
                                    *state = ExecutorState::Finishing(total);
                                    return Some(r)
                                } else {
                                    idx -= 1;
                                    r
                                }
                            },
                            Response::Future(fut) => {
                                *state = ExecutorState::Processing(idx, total);
                                self.fut = Some(fut);
                                return None
                            },
                        };
                    }
                }
                _ => Some(resp)
            }
        } else {
            Some(resp)
        }
    }
}
