use std::{io, mem};
use std::rc::Rc;
use std::cell::RefCell;

use futures::{Async, Poll, Future, Stream};
use futures::task::{Task as FutureTask, current as current_task};

use body::{Body, BodyStream};
use context::{Frame, IoContext};
use error::{Error, UnexpectedTaskFrame};
use route::{Reply, ReplyItem};
use h1writer::{Writer, WriterState};
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use middlewares::{Middleware, Finished, Started, Response};

type Handler = Fn(HttpRequest) -> Reply;
pub(crate) type PipelineHandler<'a> = &'a Fn(HttpRequest) -> Reply;

pub struct Pipeline(PipelineState);

enum PipelineState {
    None,
    Error,
    Starting(StartMiddlewares),
    Handler(WaitingResponse),
    RunMiddlewares(RunMiddlewares),
    Response(ProcessResponse),
    Finishing(FinishingMiddlewares),
    Completed(Completed),
}

impl PipelineState {

    fn is_done(&self) -> bool {
        match *self {
            PipelineState::None | PipelineState::Error
                | PipelineState::Starting(_) | PipelineState::Handler(_)
                | PipelineState::RunMiddlewares(_) | PipelineState::Response(_) => true,
            PipelineState::Finishing(ref st) => st.info.context.is_none(),
            PipelineState::Completed(_) => false,
        }
    }

    fn disconnect(&mut self) {
        let info = match *self {
            PipelineState::None | PipelineState::Error => return,
            PipelineState::Starting(ref mut st) => &mut st.info,
            PipelineState::Handler(ref mut st) => &mut st.info,
            PipelineState::RunMiddlewares(ref mut st) => &mut st.info,
            PipelineState::Response(ref mut st) => &mut st.info,
            PipelineState::Finishing(ref mut st) => &mut st.info,
            PipelineState::Completed(ref mut st) => &mut st.0,
        };
        if let Some(ref mut context) = info.context {
            context.disconnected();
        }
    }

    fn error(&mut self) -> Option<Error> {
        let info = match *self {
            PipelineState::None | PipelineState::Error => return None,
            PipelineState::Starting(ref mut st) => &mut st.info,
            PipelineState::Handler(ref mut st) => &mut st.info,
            PipelineState::RunMiddlewares(ref mut st) => &mut st.info,
            PipelineState::Response(ref mut st) => &mut st.info,
            PipelineState::Finishing(ref mut st) => &mut st.info,
            PipelineState::Completed(ref mut st) => &mut st.0,
        };
        info.error.take()
    }
}

struct PipelineInfo {
    req: HttpRequest,
    count: usize,
    mws: Rc<Vec<Box<Middleware>>>,
    context: Option<Box<IoContext>>,
    error: Option<Error>,
}

impl PipelineInfo {
    fn new(req: HttpRequest) -> PipelineInfo {
        PipelineInfo {
            req: req,
            count: 0,
            mws: Rc::new(Vec::new()),
            error: None,
            context: None,
        }
    }

    #[cfg_attr(feature = "cargo-clippy", allow(mut_from_ref))]
    fn req_mut(&self) -> &mut HttpRequest {
        #[allow(mutable_transmutes)]
        unsafe{mem::transmute(&self.req)}
    }

    fn poll_context(&mut self) -> Poll<(), Error> {
        if let Some(ref mut context) = self.context {
            match context.poll() {
                Err(err) => Err(err),
                Ok(Async::NotReady) => Ok(Async::NotReady),
                Ok(Async::Ready(_)) => Ok(Async::Ready(())),
            }
        } else {
            Ok(Async::Ready(()))
        }
    }
}

enum PipelineResponse {
    None,
    Context(Box<IoContext>),
    Response(Box<Future<Item=HttpResponse, Error=Error>>),
}

/// Future that resolves when all buffered data get sent
#[doc(hidden)]
#[derive(Debug)]
pub struct DrainFut {
    drained: bool,
    task: Option<FutureTask>,
}

impl Default for DrainFut {

    fn default() -> DrainFut {
        DrainFut {
            drained: false,
            task: None,
        }
    }
}

impl DrainFut {

    fn set(&mut self) {
        self.drained = true;
        if let Some(task) = self.task.take() {
            task.notify()
        }
    }
}

impl Future for DrainFut {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<(), ()> {
        if self.drained {
            Ok(Async::Ready(()))
        } else {
            self.task = Some(current_task());
            Ok(Async::NotReady)
        }
    }
}


impl Pipeline {

    pub fn new(req: HttpRequest,
               mw: Rc<Vec<Box<Middleware>>>,
               handler: PipelineHandler) -> Pipeline
    {
        Pipeline(StartMiddlewares::init(mw, req, handler))
    }

    pub fn error<R: Into<HttpResponse>>(err: R) -> Self {
        Pipeline(ProcessResponse::init(
            Box::new(PipelineInfo::new(HttpRequest::default())), err.into()))
    }

    pub(crate) fn disconnected(&mut self) {
        self.0.disconnect()
    }

    pub(crate) fn poll_io<T: Writer>(&mut self, io: &mut T) -> Poll<bool, Error> {
        loop {
            let state = mem::replace(&mut self.0, PipelineState::None);
            match state {
                PipelineState::None =>
                    return Ok(Async::Ready(true)),
                PipelineState::Error =>
                    return Err(io::Error::new(io::ErrorKind::Other, "Internal error").into()),
                PipelineState::Starting(st) => {
                    match st.poll() {
                        Ok(state) =>
                            self.0 = state,
                        Err(state) => {
                            self.0 = state;
                            return Ok(Async::NotReady)
                        }
                    }
                }
                PipelineState::Handler(st) => {
                    match st.poll() {
                        Ok(state) =>
                            self.0 = state,
                        Err(state) => {
                            self.0 = state;
                            return Ok(Async::NotReady)
                        }
                    }
                }
                PipelineState::RunMiddlewares(st) => {
                    match st.poll() {
                        Ok(state) =>
                            self.0 = state,
                        Err(state) => {
                            self.0 = state;
                            return Ok(Async::NotReady)
                        }
                    }
                }
                PipelineState::Response(st) => {
                    match st.poll_io(io) {
                        Ok(state) => {
                            self.0 = state;
                            if let Some(error) = self.0.error() {
                                return Err(error)
                            } else {
                                return Ok(Async::Ready(self.0.is_done()))
                            }
                        }
                        Err(state) => {
                            self.0 = state;
                            return Ok(Async::NotReady)
                        }
                    }
                }
                PipelineState::Finishing(st) => {
                    match st.poll() {
                        Ok(state) =>
                            self.0 = state,
                        Err(state) => {
                            self.0 = state;
                            return Ok(Async::NotReady)
                        }
                    }
                }
                PipelineState::Completed(st) => {
                    match st.poll() {
                        Ok(state) => {
                            self.0 = state;
                            return Ok(Async::Ready(true));
                        }
                        Err(state) => {
                            self.0 = state;
                            return Ok(Async::NotReady)
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn poll(&mut self) -> Poll<(), Error> {
        loop {
            let state = mem::replace(&mut self.0, PipelineState::None);
            match state {
                PipelineState::None | PipelineState::Error => {
                    return Ok(Async::Ready(()))
                }
                PipelineState::Starting(st) => {
                    match st.poll() {
                        Ok(state) =>
                            self.0 = state,
                        Err(state) => {
                            self.0 = state;
                            return Ok(Async::NotReady)
                        }
                    }
                }
                PipelineState::Handler(st) => {
                    match st.poll() {
                        Ok(state) =>
                            self.0 = state,
                        Err(state) => {
                            self.0 = state;
                            return Ok(Async::NotReady)
                        }
                    }
                }
                PipelineState::RunMiddlewares(st) => {
                    match st.poll() {
                        Ok(state) =>
                            self.0 = state,
                        Err(state) => {
                            self.0 = state;
                            return Ok(Async::NotReady)
                        }
                    }
                }
                PipelineState::Response(_) => {
                    self.0 = state;
                    return Ok(Async::NotReady);
                }
                PipelineState::Finishing(st) => {
                    match st.poll() {
                        Ok(state) =>
                            self.0 = state,
                        Err(state) => {
                            self.0 = state;
                            return Ok(Async::NotReady)
                        }
                    }
                }
                PipelineState::Completed(st) => {
                    match st.poll() {
                        Ok(state) => {
                            self.0 = state;
                            return Ok(Async::Ready(()));
                        }
                        Err(state) => {
                            self.0 = state;
                            return Ok(Async::NotReady)
                        }
                    }
                }
            }
        }
    }
}

type Fut = Box<Future<Item=Option<HttpResponse>, Error=Error>>;

/// Middlewares start executor
struct StartMiddlewares {
    hnd: *mut Handler,
    fut: Option<Fut>,
    info: Box<PipelineInfo>,
}

impl StartMiddlewares {

    fn init(mws: Rc<Vec<Box<Middleware>>>,
            req: HttpRequest, handler: PipelineHandler) -> PipelineState {
        let mut info = PipelineInfo {
            req: req,
            count: 0,
            mws: mws,
            error: None,
            context: None,
        };

        // execute middlewares, we need this stage because middlewares could be non-async
        // and we can move to next state immidietly
        let len = info.mws.len();
        loop {
            if info.count == len {
                let reply = (&*handler)(info.req.clone());
                return WaitingResponse::init(Box::new(info), reply)
            } else {
                match info.mws[info.count].start(&mut info.req) {
                    Started::Done =>
                        info.count += 1,
                    Started::Response(resp) =>
                        return RunMiddlewares::init(Box::new(info), resp),
                    Started::Future(mut fut) =>
                        match fut.poll() {
                            Ok(Async::NotReady) =>
                                return PipelineState::Starting(StartMiddlewares {
                                    hnd: handler as *const _ as *mut _,
                                    fut: Some(fut),
                                    info: Box::new(info)}),
                            Ok(Async::Ready(resp)) => {
                                if let Some(resp) = resp {
                                    return RunMiddlewares::init(Box::new(info), resp);
                                }
                                info.count += 1;
                            }
                            Err(err) =>
                                return ProcessResponse::init(Box::new(info), err.into()),
                        },
                    Started::Err(err) =>
                        return ProcessResponse::init(Box::new(info), err.into()),
                }
            }
        }
    }

    fn poll(mut self) -> Result<PipelineState, PipelineState> {
        let len = self.info.mws.len();
        'outer: loop {
            match self.fut.as_mut().unwrap().poll() {
                Ok(Async::NotReady) =>
                    return Err(PipelineState::Starting(self)),
                Ok(Async::Ready(resp)) => {
                    self.info.count += 1;
                    if let Some(resp) = resp {
                        return Ok(RunMiddlewares::init(self.info, resp));
                    }
                    if self.info.count == len {
                        let reply = (unsafe{&*self.hnd})(self.info.req.clone());
                        return Ok(WaitingResponse::init(self.info, reply));
                    } else {
                        loop {
                            match self.info.mws[self.info.count].start(self.info.req_mut()) {
                                Started::Done =>
                                    self.info.count += 1,
                                Started::Response(resp) => {
                                    return Ok(RunMiddlewares::init(self.info, resp));
                                },
                                Started::Future(fut) => {
                                    self.fut = Some(fut);
                                    continue 'outer
                                },
                                Started::Err(err) =>
                                    return Ok(ProcessResponse::init(self.info, err.into()))
                            }
                        }
                    }
                }
                Err(err) =>
                    return Ok(ProcessResponse::init(self.info, err.into()))
            }
        }
    }
}

// waiting for response
struct WaitingResponse {
    info: Box<PipelineInfo>,
    stream: PipelineResponse,
}

impl WaitingResponse {

    fn init(info: Box<PipelineInfo>, reply: Reply) -> PipelineState
    {
        let stream = match reply.into() {
            ReplyItem::Message(resp) =>
                return RunMiddlewares::init(info, resp),
            ReplyItem::Actor(ctx) =>
                PipelineResponse::Context(ctx),
            ReplyItem::Future(fut) =>
                PipelineResponse::Response(fut),
        };

        PipelineState::Handler(
            WaitingResponse { info: info, stream: stream })
    }

    fn poll(mut self) -> Result<PipelineState, PipelineState> {
        let stream = mem::replace(&mut self.stream, PipelineResponse::None);

        match stream {
            PipelineResponse::Context(mut context) => {
                loop {
                    match context.poll() {
                        Ok(Async::Ready(Some(frame))) => {
                            match frame {
                                Frame::Message(resp) => {
                                    self.info.context = Some(context);
                                    return Ok(RunMiddlewares::init(self.info, resp))
                                }
                                Frame::Payload(_) | Frame::Drain(_) => (),
                            }
                        },
                        Ok(Async::Ready(None)) => {
                            error!("Unexpected eof");
                            let err: Error = UnexpectedTaskFrame.into();
                            return Ok(ProcessResponse::init(self.info, err.into()))
                        },
                        Ok(Async::NotReady) => {
                            self.stream = PipelineResponse::Context(context);
                            return Err(PipelineState::Handler(self))
                        },
                        Err(err) =>
                            return Ok(ProcessResponse::init(self.info, err.into()))
                    }
                }
            },
            PipelineResponse::Response(mut fut) => {
                match fut.poll() {
                    Ok(Async::NotReady) => {
                        self.stream = PipelineResponse::Response(fut);
                        Err(PipelineState::Handler(self))
                    }
                    Ok(Async::Ready(response)) =>
                        Ok(RunMiddlewares::init(self.info, response)),
                    Err(err) =>
                        Ok(ProcessResponse::init(self.info, err.into())),
                }
            }
            PipelineResponse::None => {
                unreachable!("Broken internal state")
            }
        }

    }
}

/// Middlewares response executor
pub(crate) struct RunMiddlewares {
    info: Box<PipelineInfo>,
    curr: usize,
    fut: Option<Box<Future<Item=HttpResponse, Error=Error>>>,
}

impl RunMiddlewares {

    fn init(mut info: Box<PipelineInfo>, mut resp: HttpResponse) -> PipelineState
    {
        if info.count == 0 {
            return ProcessResponse::init(info, resp);
        }
        let mut curr = 0;
        let len = info.mws.len();

        loop {
            resp = match info.mws[curr].response(info.req_mut(), resp) {
                Response::Err(err) => {
                    info.count = curr + 1;
                    return ProcessResponse::init(info, err.into())
                }
                Response::Done(r) => {
                    curr += 1;
                    if curr == len {
                        return ProcessResponse::init(info, r)
                    } else {
                        r
                    }
                },
                Response::Future(fut) => {
                    return PipelineState::RunMiddlewares(
                        RunMiddlewares { info: info, curr: curr, fut: Some(fut) })
                },
            };
        }
    }

    fn poll(mut self) -> Result<PipelineState, PipelineState> {
        let len = self.info.mws.len();

        loop {
            // poll latest fut
            let mut resp = match self.fut.as_mut().unwrap().poll() {
                Ok(Async::NotReady) =>
                    return Ok(PipelineState::RunMiddlewares(self)),
                Ok(Async::Ready(resp)) => {
                    self.curr += 1;
                    resp
                }
                Err(err) =>
                    return Ok(ProcessResponse::init(self.info, err.into())),
            };

            loop {
                if self.curr == len {
                    return Ok(ProcessResponse::init(self.info, resp));
                } else {
                    match self.info.mws[self.curr].response(self.info.req_mut(), resp) {
                        Response::Err(err) =>
                            return Ok(ProcessResponse::init(self.info, err.into())),
                        Response::Done(r) => {
                            self.curr += 1;
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

struct ProcessResponse {
    resp: HttpResponse,
    iostate: IOState,
    running: RunningState,
    drain: DrainVec,
    info: Box<PipelineInfo>,
}

#[derive(PartialEq)]
enum RunningState {
    Running,
    Paused,
    Done,
}

impl RunningState {
    #[inline]
    fn pause(&mut self) {
        if *self != RunningState::Done {
            *self = RunningState::Paused
        }
    }
    #[inline]
    fn resume(&mut self) {
        if *self != RunningState::Done {
            *self = RunningState::Running
        }
    }
}

enum IOState {
    Response,
    Payload(BodyStream),
    Context,
    Done,
}

impl IOState {
    fn is_done(&self) -> bool {
        match *self {
            IOState::Done => true,
            _ => false
        }
    }
}

struct DrainVec(Vec<Rc<RefCell<DrainFut>>>);
impl Drop for DrainVec {
    fn drop(&mut self) {
        for drain in &mut self.0 {
            drain.borrow_mut().set()
        }
    }
}

impl ProcessResponse {

    fn init(info: Box<PipelineInfo>, resp: HttpResponse) -> PipelineState
    {
        PipelineState::Response(
            ProcessResponse{ resp: resp,
                             iostate: IOState::Response,
                             running: RunningState::Running,
                             drain: DrainVec(Vec::new()),
                             info: info})
    }

    fn poll_io<T: Writer>(mut self, io: &mut T) -> Result<PipelineState, PipelineState> {
        if self.drain.0.is_empty() && self.running != RunningState::Paused {
            // if task is paused, write buffer is probably full

            loop {
                let result = match mem::replace(&mut self.iostate, IOState::Done) {
                    IOState::Response => {
                        let result = match io.start(self.info.req_mut(), &mut self.resp) {
                            Ok(res) => res,
                            Err(err) => {
                                self.info.error = Some(err.into());
                                return Ok(FinishingMiddlewares::init(self.info, self.resp))
                            }
                        };

                        match self.resp.replace_body(Body::Empty) {
                            Body::Streaming(stream) | Body::Upgrade(stream) =>
                                self.iostate = IOState::Payload(stream),
                            Body::StreamingContext | Body::UpgradeContext =>
                                self.iostate = IOState::Context,
                            _ => (),
                        }

                        result
                    },
                    IOState::Payload(mut body) => {
                        // always poll context
                        if self.running == RunningState::Running {
                            match self.info.poll_context() {
                                Ok(Async::NotReady) => (),
                                Ok(Async::Ready(_)) =>
                                    self.running = RunningState::Done,
                                Err(err) => {
                                    self.info.error = Some(err);
                                    return Ok(FinishingMiddlewares::init(self.info, self.resp))
                                }
                            }
                        }

                        match body.poll() {
                            Ok(Async::Ready(None)) => {
                                self.iostate = IOState::Done;
                                if let Err(err) = io.write_eof() {
                                    self.info.error = Some(err.into());
                                    return Ok(FinishingMiddlewares::init(self.info, self.resp))
                                }
                                break
                            },
                            Ok(Async::Ready(Some(chunk))) => {
                                self.iostate = IOState::Payload(body);
                                match io.write(chunk.as_ref()) {
                                    Err(err) => {
                                        self.info.error = Some(err.into());
                                        return Ok(FinishingMiddlewares::init(
                                            self.info, self.resp))
                                    },
                                    Ok(result) => result
                                }
                            }
                            Ok(Async::NotReady) => {
                                self.iostate = IOState::Payload(body);
                                break
                            },
                            Err(err) => {
                                self.info.error = Some(err);
                                return Ok(FinishingMiddlewares::init(self.info, self.resp))
                            }
                        }
                    },
                    IOState::Context => {
                        match self.info.context.as_mut().unwrap().poll() {
                            Ok(Async::Ready(Some(frame))) => {
                                match frame {
                                    Frame::Message(msg) => {
                                        error!("Unexpected message frame {:?}", msg);
                                        self.info.error = Some(UnexpectedTaskFrame.into());
                                        return Ok(
                                            FinishingMiddlewares::init(self.info, self.resp))
                                    },
                                    Frame::Payload(None) => {
                                        self.iostate = IOState::Done;
                                        if let Err(err) = io.write_eof() {
                                            self.info.error = Some(err.into());
                                            return Ok(
                                                FinishingMiddlewares::init(self.info, self.resp))
                                        }
                                        break
                                    },
                                    Frame::Payload(Some(chunk)) => {
                                        self.iostate = IOState::Context;
                                        match io.write(chunk.as_ref()) {
                                            Err(err) => {
                                                self.info.error = Some(err.into());
                                                return Ok(FinishingMiddlewares::init(
                                                    self.info, self.resp))
                                            },
                                            Ok(result) => result
                                        }
                                    },
                                    Frame::Drain(fut) => {
                                        self.drain.0.push(fut);
                                        break
                                    }
                                }
                            },
                            Ok(Async::Ready(None)) => {
                                self.iostate = IOState::Done;
                                self.info.context.take();
                                break
                            }
                            Ok(Async::NotReady) => {
                                self.iostate = IOState::Context;
                                break
                            }
                            Err(err) => {
                                self.info.error = Some(err);
                                return Ok(FinishingMiddlewares::init(self.info, self.resp))
                            }
                        }
                    }
                    IOState::Done => break,
                };

                match result {
                    WriterState::Pause => {
                        self.running.pause();
                        break
                    }
                    WriterState::Done => {
                        self.running.resume()
                    },
                }
            }
        }

        // flush io
        match io.poll_complete() {
            Ok(Async::Ready(_)) =>
                self.running.resume(),
            Ok(Async::NotReady) =>
                return Err(PipelineState::Response(self)),
            Err(err) => {
                debug!("Error sending data: {}", err);
                self.info.error = Some(err.into());
                return Ok(FinishingMiddlewares::init(self.info, self.resp))
            }
        }

        // drain futures
        if !self.drain.0.is_empty() {
            for fut in &mut self.drain.0 {
                fut.borrow_mut().set()
            }
            self.drain.0.clear();
        }

        // response is completed
        if self.iostate.is_done() {
            self.resp.set_response_size(io.written());
            Ok(FinishingMiddlewares::init(self.info, self.resp))
        } else {
            Err(PipelineState::Response(self))
        }
    }
}

/// Middlewares start executor
struct FinishingMiddlewares {
    info: Box<PipelineInfo>,
    resp: HttpResponse,
    fut: Option<Box<Future<Item=(), Error=Error>>>,
}

impl FinishingMiddlewares {

    fn init(info: Box<PipelineInfo>, resp: HttpResponse) -> PipelineState {
        if info.count == 0 {
            Completed::init(info)
        } else {
            match (FinishingMiddlewares{info: info, resp: resp, fut: None}).poll() {
                Ok(st) | Err(st) => st,
            }
        }
    }

    fn poll(mut self) -> Result<PipelineState, PipelineState> {
        loop {
            // poll latest fut
            let not_ready = if let Some(ref mut fut) = self.fut {
                match fut.poll() {
                    Ok(Async::NotReady) => {
                        true
                    },
                    Ok(Async::Ready(())) => {
                        false
                    },
                    Err(err) => {
                        error!("Middleware finish error: {}", err);
                        false
                    }
                }
            } else {
                false
            };
            if not_ready {
                return Ok(PipelineState::Finishing(self))
            }
            self.fut = None;
            self.info.count -= 1;

            match self.info.mws[self.info.count].finish(self.info.req_mut(), &self.resp) {
                Finished::Done => {
                    if self.info.count == 0 {
                        return Ok(Completed::init(self.info))
                    }
                }
                Finished::Future(fut) => {
                    self.fut = Some(fut);
                },
            }
        }
    }
}

struct Completed(Box<PipelineInfo>);

impl Completed {

    fn init(info: Box<PipelineInfo>) -> PipelineState {
        if info.context.is_none() {
            PipelineState::None
        } else {
            PipelineState::Completed(Completed(info))
        }
    }

    fn poll(mut self) -> Result<PipelineState, PipelineState> {
        match self.0.poll_context() {
            Ok(Async::NotReady) => Ok(PipelineState::Completed(self)),
            Ok(Async::Ready(())) => Ok(PipelineState::None),
            Err(_) => Ok(PipelineState::Error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix::*;
    use context::HttpContext;
    use tokio_core::reactor::Core;
    use futures::future::{lazy, result};

    impl PipelineState {
        fn is_none(&self) -> Option<bool> {
            if let PipelineState::None = *self { Some(true) } else { None }
        }
        fn completed(self) -> Option<Completed> {
            if let PipelineState::Completed(c) = self { Some(c) } else { None }
        }
    }

    struct MyActor;
    impl Actor for MyActor {
        type Context = HttpContext<MyActor>;
    }

    #[test]
    fn test_completed() {
        Core::new().unwrap().run(lazy(|| {
            let info = Box::new(PipelineInfo::new(HttpRequest::default()));
            Completed::init(info).is_none().unwrap();

            let req = HttpRequest::default();
            let mut ctx = HttpContext::new(req.clone(), MyActor);
            let addr: Address<_> = ctx.address();
            let mut info = Box::new(PipelineInfo::new(req));
            info.context = Some(Box::new(ctx));
            let mut state = Completed::init(info).completed().unwrap();

            let st = state.poll().ok().unwrap();
            assert!(!st.is_done());

            state = st.completed().unwrap();
            drop(addr);

            state.poll().ok().unwrap().is_none().unwrap();

            result(Ok::<_, ()>(()))
        })).unwrap()
    }
}
