use std::mem;
use std::rc::Rc;

use futures::{Async, Poll, Future};

use task::Task;
use route::Reply;
use error::Error;
use h1writer::Writer;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use middlewares::{Middleware, Finished, Started, Response};

type Handler = Fn(HttpRequest) -> Reply;
pub(crate) type PipelineHandler<'a> = &'a Fn(HttpRequest) -> Reply;

pub struct Pipeline(PipelineState);

enum PipelineState {
    None,
    Starting(Start),
    Handle(Box<Handle>),
    Finishing(Box<Finish>),
    Error(Box<(Task, HttpRequest)>),
    Task(Box<(Task, HttpRequest)>),
}

impl Pipeline {

    pub fn new(req: HttpRequest, mw: Rc<Vec<Box<Middleware>>>, handler: PipelineHandler) -> Pipeline
    {
        if mw.is_empty() {
            let task = Task::new((handler)(req.clone()));
            Pipeline(PipelineState::Task(Box::new((task, req))))
        } else {
            match Start::init(mw, req, handler) {
                Ok(StartResult::Ready(res)) =>
                    Pipeline(PipelineState::Handle(res)),
                Ok(StartResult::NotReady(res)) =>
                    Pipeline(PipelineState::Starting(res)),
                Err(err) =>
                    Pipeline(PipelineState::Error(
                        Box::new((Task::from_error(err), HttpRequest::default()))))
            }
        }
    }

    pub fn error<R: Into<HttpResponse>>(resp: R) -> Self {
        Pipeline(PipelineState::Error(
            Box::new((Task::from_response(resp), HttpRequest::default()))))
    }

    pub(crate) fn disconnected(&mut self) {
        match self.0 {
            PipelineState::Starting(ref mut st) =>
                st.disconnected(),
            PipelineState::Handle(ref mut st) =>
                st.task.disconnected(),
            PipelineState::Task(ref mut st) | PipelineState::Error(ref mut st) =>
                st.0.disconnected(),
            _ =>(),
        }
    }

    pub(crate) fn poll_io<T: Writer>(&mut self, io: &mut T) -> Poll<bool, Error> {
        loop {
            match mem::replace(&mut self.0, PipelineState::None) {
                PipelineState::Task(mut st) => {
                    let req:&mut HttpRequest = unsafe{mem::transmute(&mut st.1)};
                    let res = st.0.poll_io(io, req);
                    self.0 = PipelineState::Task(st);
                    return res
                }
                PipelineState::Starting(mut st) => {
                    match st.poll() {
                        Ok(Async::NotReady) => {
                            self.0 = PipelineState::Starting(st);
                            return Ok(Async::NotReady)
                        }
                        Ok(Async::Ready(h)) =>
                            self.0 = PipelineState::Handle(h),
                        Err(err) =>
                            self.0 = PipelineState::Error(
                                Box::new((Task::from_error(err), HttpRequest::default())))
                    }
                }
                PipelineState::Handle(mut st) => {
                    let res = st.poll_io(io);
                    if let Ok(Async::Ready(r)) = res {
                        if r {
                            self.0 = PipelineState::Finishing(st.finish());
                            return Ok(Async::Ready(false))
                        } else {
                            self.0 = PipelineState::Handle(st);
                            return res
                        }
                    } else {
                        self.0 = PipelineState::Handle(st);
                        return res
                    }
                }
                PipelineState::Error(mut st) => {
                    let req:&mut HttpRequest = unsafe{mem::transmute(&mut st.1)};
                    let res = st.0.poll_io(io, req);
                    self.0 = PipelineState::Error(st);
                    return res
                }
                PipelineState::Finishing(_) | PipelineState::None => unreachable!(),
            }
        }
    }

    pub(crate) fn poll(&mut self) -> Poll<(), Error> {
        loop {
            match mem::replace(&mut self.0, PipelineState::None) {
                PipelineState::Handle(mut st) => {
                    let res = st.poll();
                    match res {
                        Ok(Async::NotReady) => {
                            self.0 = PipelineState::Handle(st);
                            return Ok(Async::NotReady)
                        }
                        Ok(Async::Ready(())) | Err(_) => {
                            self.0 = PipelineState::Finishing(st.finish());
                        }
                    }
                }
                PipelineState::Finishing(mut st) => {
                    let res = st.poll();
                    self.0 = PipelineState::Finishing(st);
                    return Ok(res)
                }
                PipelineState::Error(mut st) => {
                    let res = st.0.poll();
                    self.0 = PipelineState::Error(st);
                    return res
                }
                PipelineState::Task(mut st) => {
                    let res = st.0.poll();
                    self.0 = PipelineState::Task(st);
                    return res
                }
                _ => {
                    return Ok(Async::Ready(()))
                }
            }
        }
    }
}

type Fut = Box<Future<Item=Option<HttpResponse>, Error=Error>>;

/// Middlewares start executor
struct Start {
    idx: usize,
    hnd: *mut Handler,
    disconnected: bool,
    req: HttpRequest,
    fut: Option<Fut>,
    middlewares: Rc<Vec<Box<Middleware>>>,
}

enum StartResult {
    Ready(Box<Handle>),
    NotReady(Start),
}

impl Start {

    fn init(mw: Rc<Vec<Box<Middleware>>>,
            req: HttpRequest, handler: PipelineHandler) -> Result<StartResult, Error> {
        Start {
            idx: 0,
            fut: None,
            req: req,
            disconnected: false,
            hnd: handler as *const _ as *mut _,
            middlewares: mw,
        }.start()
    }

    fn disconnected(&mut self) {
        self.disconnected = true;
    }

    fn prepare(&self, mut task: Task) -> Task {
        if self.disconnected {
            task.disconnected()
        }
        task.set_middlewares(MiddlewaresResponse::new(self.idx-1, Rc::clone(&self.middlewares)));
        task
    }

    fn start(mut self) -> Result<StartResult, Error> {
        let len = self.middlewares.len();
        loop {
            if self.idx == len {
                let task = Task::new((unsafe{&*self.hnd})(self.req.clone()));
                return Ok(StartResult::Ready(
                    Box::new(Handle::new(self.idx-1, self.req.clone(),
                                         self.prepare(task), self.middlewares))))
            } else {
                match self.middlewares[self.idx].start(&mut self.req) {
                    Started::Done =>
                        self.idx += 1,
                    Started::Response(resp) =>
                        return Ok(StartResult::Ready(
                            Box::new(Handle::new(
                                self.idx, self.req.clone(),
                                self.prepare(Task::from_response(resp)), self.middlewares)))),
                    Started::Future(mut fut) =>
                        match fut.poll() {
                            Ok(Async::NotReady) => {
                                self.fut = Some(fut);
                                return Ok(StartResult::NotReady(self))
                            }
                            Ok(Async::Ready(resp)) => {
                                if let Some(resp) = resp {
                                    return Ok(StartResult::Ready(
                                        Box::new(Handle::new(
                                            self.idx, self.req.clone(),
                                            self.prepare(Task::from_response(resp)),
                                            self.middlewares))))
                                }
                                self.idx += 1;
                            }
                            Err(err) => return Err(err)
                        },
                    Started::Err(err) => return Err(err),
                }
            }
        }
    }

    fn poll(&mut self) -> Poll<Box<Handle>, Error> {
        let len = self.middlewares.len();
        'outer: loop {
            match self.fut.as_mut().unwrap().poll() {
                Ok(Async::NotReady) => return Ok(Async::NotReady),
                Ok(Async::Ready(resp)) => {
                    self.idx += 1;
                    if let Some(resp) = resp {
                        return Ok(Async::Ready(Box::new(Handle::new(
                            self.idx-1, self.req.clone(),
                            self.prepare(Task::from_response(resp)),
                            Rc::clone(&self.middlewares)))))
                    }
                    if self.idx == len {
                        let task = Task::new((unsafe{&*self.hnd})(self.req.clone()));
                        return Ok(Async::Ready(Box::new(Handle::new(
                            self.idx-1, self.req.clone(),
                            self.prepare(task), Rc::clone(&self.middlewares)))))
                    } else {
                        loop {
                            match self.middlewares[self.idx].start(&mut self.req) {
                                Started::Done =>
                                    self.idx += 1,
                                Started::Response(resp) => {
                                    self.idx += 1;
                                    return Ok(Async::Ready(Box::new(Handle::new(
                                        self.idx-1, self.req.clone(),
                                        self.prepare(Task::from_response(resp)),
                                        Rc::clone(&self.middlewares)))))
                                },
                                Started::Future(fut) => {
                                    self.fut = Some(fut);
                                    continue 'outer
                                },
                                Started::Err(err) => return Err(err),
                            }
                        }
                    }
                }
                Err(err) => return Err(err)
            }
        }
    }
}

struct Handle {
    idx: usize,
    req: HttpRequest,
    task: Task,
    middlewares: Rc<Vec<Box<Middleware>>>,
}

impl Handle {
    fn new(idx: usize, req: HttpRequest, task: Task, mw: Rc<Vec<Box<Middleware>>>) -> Handle {
        Handle { idx: idx, req: req, task:task, middlewares: mw }
    }

    fn poll_io<T: Writer>(&mut self, io: &mut T) -> Poll<bool, Error> {
        self.task.poll_io(io, &mut self.req)
    }

    fn poll(&mut self) -> Poll<(), Error> {
        self.task.poll()
    }

    fn finish(mut self) -> Box<Finish> {
        Box::new(Finish {
            idx: self.idx,
            req: self.req,
            fut: None,
            resp: self.task.response(),
            middlewares: self.middlewares
        })
    }
}

/// Middlewares start executor
struct Finish {
    idx: usize,
    req: HttpRequest,
    resp: HttpResponse,
    fut: Option<Box<Future<Item=(), Error=Error>>>,
    middlewares: Rc<Vec<Box<Middleware>>>,
}

impl Finish {

    pub fn poll(&mut self) -> Async<()> {
        loop {
            // poll latest fut
            if let Some(ref mut fut) = self.fut {
                match fut.poll() {
                    Ok(Async::NotReady) => return Async::NotReady,
                    Ok(Async::Ready(())) => self.idx -= 1,
                    Err(err) => {
                        error!("Middleware finish error: {}", err);
                        self.idx -= 1;
                    }
                }
            }
            self.fut = None;

            match self.middlewares[self.idx].finish(&mut self.req, &self.resp) {
                Finished::Done => {
                    if self.idx == 0 {
                        return Async::Ready(())
                    } else {
                        self.idx -= 1
                    }
                }
                Finished::Future(fut) => {
                    self.fut = Some(fut);
                },
            }
        }
    }
}

/// Middlewares response executor
pub(crate) struct MiddlewaresResponse {
    idx: usize,
    fut: Option<Box<Future<Item=HttpResponse, Error=Error>>>,
    middlewares: Rc<Vec<Box<Middleware>>>,
}

impl MiddlewaresResponse {

    fn new(idx: usize, mw: Rc<Vec<Box<Middleware>>>) -> MiddlewaresResponse {
        MiddlewaresResponse {
            idx: idx,
            fut: None,
            middlewares: mw }
    }

    pub fn response(&mut self, req: &mut HttpRequest, mut resp: HttpResponse)
                    -> Result<Option<HttpResponse>, Error>
    {
        loop {
            resp = match self.middlewares[self.idx].response(req, resp) {
                Response::Err(err) =>
                    return Err(err),
                Response::Done(r) => {
                    if self.idx == 0 {
                        return Ok(Some(r))
                    } else {
                        self.idx -= 1;
                        r
                    }
                },
                Response::Future(fut) => {
                    self.fut = Some(fut);
                    return Ok(None)
                },
            };
        }
    }

    pub fn poll(&mut self, req: &mut HttpRequest) -> Poll<HttpResponse, Error> {
        loop {
            // poll latest fut
            let mut resp = match self.fut.as_mut().unwrap().poll() {
                Ok(Async::NotReady) =>
                    return Ok(Async::NotReady),
                Ok(Async::Ready(resp)) => {
                    self.idx -= 1;
                    resp
                }
                Err(err) => return Err(err)
            };

            loop {
                if self.idx == 0 {
                    return Ok(Async::Ready(resp))
                } else {
                    match self.middlewares[self.idx].response(req, resp) {
                        Response::Err(err) =>
                            return Err(err),
                        Response::Done(r) => {
                            self.idx -= 1;
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
}
