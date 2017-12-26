use std::{io, mem};
use std::rc::Rc;
use std::marker::PhantomData;

use futures::{Async, Poll, Future, Stream};
use futures::unsync::oneshot;

use channel::HttpHandlerTask;
use body::{Body, BodyStream};
use context::{Frame, IoContext};
use error::{Error, UnexpectedTaskFrame};
use handler::{Reply, ReplyItem};
use h1writer::{Writer, WriterState};
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use middlewares::{Middleware, Finished, Started, Response};

type Handler<S> = FnMut(HttpRequest<S>) -> Reply;
pub(crate) type PipelineHandler<'a, S> = &'a mut FnMut(HttpRequest<S>) -> Reply;

pub struct Pipeline<S>(PipelineInfo<S>, PipelineState<S>);

enum PipelineState<S> {
    None,
    Error,
    Starting(StartMiddlewares<S>),
    Handler(WaitingResponse<S>),
    RunMiddlewares(RunMiddlewares<S>),
    Response(ProcessResponse<S>),
    Finishing(FinishingMiddlewares<S>),
    Completed(Completed<S>),
}

struct PipelineInfo<S> {
    req: HttpRequest<S>,
    count: usize,
    mws: Rc<Vec<Box<Middleware<S>>>>,
    context: Option<Box<IoContext>>,
    error: Option<Error>,
}

impl<S> PipelineInfo<S> {
    fn new(req: HttpRequest<S>) -> PipelineInfo<S> {
        PipelineInfo {
            req: req,
            count: 0,
            mws: Rc::new(Vec::new()),
            error: None,
            context: None,
        }
    }

    #[cfg_attr(feature = "cargo-clippy", allow(mut_from_ref))]
    fn req_mut(&self) -> &mut HttpRequest<S> {
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

impl<S> Pipeline<S> {

    pub fn new(req: HttpRequest<S>,
               mws: Rc<Vec<Box<Middleware<S>>>>,
               handler: PipelineHandler<S>) -> Pipeline<S>
    {
        let mut info = PipelineInfo {
            req: req,
            count: 0,
            mws: mws,
            error: None,
            context: None,
        };
        let state = StartMiddlewares::init(&mut info, handler);

        Pipeline(info, state)
    }
}

impl Pipeline<()> {
    pub fn error<R: Into<HttpResponse>>(err: R) -> Box<HttpHandlerTask> {
        Box::new(Pipeline(
            PipelineInfo::new(
                HttpRequest::default()), ProcessResponse::init(err.into())))
    }
}

impl<S> Pipeline<S> {

    fn is_done(&self) -> bool {
        match self.1 {
            PipelineState::None | PipelineState::Error
                | PipelineState::Starting(_) | PipelineState::Handler(_)
                | PipelineState::RunMiddlewares(_) | PipelineState::Response(_) => true,
            PipelineState::Finishing(_) => self.0.context.is_none(),
            PipelineState::Completed(_) => false,
        }
    }
}

impl<S> HttpHandlerTask for Pipeline<S> {

    fn disconnected(&mut self) {
        if let Some(ref mut context) = self.0.context {
            context.disconnected();
        }
    }

    fn poll_io(&mut self, io: &mut Writer) -> Poll<bool, Error> {
        loop {
            let state = mem::replace(&mut self.1, PipelineState::None);
            match state {
                PipelineState::None =>
                    return Ok(Async::Ready(true)),
                PipelineState::Error =>
                    return Err(io::Error::new(io::ErrorKind::Other, "Internal error").into()),
                PipelineState::Starting(st) => {
                    match st.poll(&mut self.0) {
                        Ok(state) =>
                            self.1 = state,
                        Err(state) => {
                            self.1 = state;
                            return Ok(Async::NotReady)
                        }
                    }
                }
                PipelineState::Handler(st) => {
                    match st.poll(&mut self.0) {
                        Ok(state) =>
                            self.1 = state,
                        Err(state) => {
                            self.1 = state;
                            return Ok(Async::NotReady)
                        }
                    }
                }
                PipelineState::RunMiddlewares(st) => {
                    match st.poll(&mut self.0) {
                        Ok(state) =>
                            self.1 = state,
                        Err(state) => {
                            self.1 = state;
                            return Ok(Async::NotReady)
                        }
                    }
                }
                PipelineState::Response(st) => {
                    match st.poll_io(io, &mut self.0) {
                        Ok(state) => {
                            self.1 = state;
                            if let Some(error) = self.0.error.take() {
                                return Err(error)
                            } else {
                                return Ok(Async::Ready(self.is_done()))
                            }
                        }
                        Err(state) => {
                            self.1 = state;
                            return Ok(Async::NotReady)
                        }
                    }
                }
                PipelineState::Finishing(st) => {
                    match st.poll(&mut self.0) {
                        Ok(state) =>
                            self.1 = state,
                        Err(state) => {
                            self.1 = state;
                            return Ok(Async::NotReady)
                        }
                    }
                }
                PipelineState::Completed(st) => {
                    match st.poll(&mut self.0) {
                        Ok(state) => {
                            self.1 = state;
                            return Ok(Async::Ready(true));
                        }
                        Err(state) => {
                            self.1 = state;
                            return Ok(Async::NotReady)
                        }
                    }
                }
            }
        }
    }

    fn poll(&mut self) -> Poll<(), Error> {
        loop {
            let state = mem::replace(&mut self.1, PipelineState::None);
            match state {
                PipelineState::None | PipelineState::Error => {
                    return Ok(Async::Ready(()))
                }
                PipelineState::Starting(st) => {
                    match st.poll(&mut self.0) {
                        Ok(state) =>
                            self.1 = state,
                        Err(state) => {
                            self.1 = state;
                            return Ok(Async::NotReady)
                        }
                    }
                }
                PipelineState::Handler(st) => {
                    match st.poll(&mut self.0) {
                        Ok(state) =>
                            self.1 = state,
                        Err(state) => {
                            self.1 = state;
                            return Ok(Async::NotReady)
                        }
                    }
                }
                PipelineState::RunMiddlewares(st) => {
                    match st.poll(&mut self.0) {
                        Ok(state) =>
                            self.1 = state,
                        Err(state) => {
                            self.1 = state;
                            return Ok(Async::NotReady)
                        }
                    }
                }
                PipelineState::Response(_) => {
                    self.1 = state;
                    return Ok(Async::NotReady);
                }
                PipelineState::Finishing(st) => {
                    match st.poll(&mut self.0) {
                        Ok(state) =>
                            self.1 = state,
                        Err(state) => {
                            self.1 = state;
                            return Ok(Async::NotReady)
                        }
                    }
                }
                PipelineState::Completed(st) => {
                    match st.poll(&mut self.0) {
                        Ok(state) => {
                            self.1 = state;
                            return Ok(Async::Ready(()));
                        }
                        Err(state) => {
                            self.1 = state;
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
struct StartMiddlewares<S> {
    hnd: *mut Handler<S>,
    fut: Option<Fut>,
}

impl<S> StartMiddlewares<S> {

    fn init(info: &mut PipelineInfo<S>, handler: PipelineHandler<S>) -> PipelineState<S> {
        // execute middlewares, we need this stage because middlewares could be non-async
        // and we can move to next state immidietly
        let len = info.mws.len();
        loop {
            if info.count == len {
                let reply = (&mut *handler)(info.req.clone());
                return WaitingResponse::init(info, reply)
            } else {
                match info.mws[info.count].start(&mut info.req) {
                    Started::Done =>
                        info.count += 1,
                    Started::Response(resp) =>
                        return RunMiddlewares::init(info, resp),
                    Started::Future(mut fut) =>
                        match fut.poll() {
                            Ok(Async::NotReady) =>
                                return PipelineState::Starting(StartMiddlewares {
                                    hnd: handler as *const _ as *mut _,
                                    fut: Some(fut)}),
                            Ok(Async::Ready(resp)) => {
                                if let Some(resp) = resp {
                                    return RunMiddlewares::init(info, resp);
                                }
                                info.count += 1;
                            }
                            Err(err) =>
                                return ProcessResponse::init(err.into()),
                        },
                    Started::Err(err) =>
                        return ProcessResponse::init(err.into()),
                }
            }
        }
    }

    fn poll(mut self, info: &mut PipelineInfo<S>) -> Result<PipelineState<S>, PipelineState<S>> {
        let len = info.mws.len();
        'outer: loop {
            match self.fut.as_mut().unwrap().poll() {
                Ok(Async::NotReady) =>
                    return Err(PipelineState::Starting(self)),
                Ok(Async::Ready(resp)) => {
                    info.count += 1;
                    if let Some(resp) = resp {
                        return Ok(RunMiddlewares::init(info, resp));
                    }
                    if info.count == len {
                        let reply = (unsafe{&mut *self.hnd})(info.req.clone());
                        return Ok(WaitingResponse::init(info, reply));
                    } else {
                        loop {
                            match info.mws[info.count].start(info.req_mut()) {
                                Started::Done =>
                                    info.count += 1,
                                Started::Response(resp) => {
                                    return Ok(RunMiddlewares::init(info, resp));
                                },
                                Started::Future(fut) => {
                                    self.fut = Some(fut);
                                    continue 'outer
                                },
                                Started::Err(err) =>
                                    return Ok(ProcessResponse::init(err.into()))
                            }
                        }
                    }
                }
                Err(err) =>
                    return Ok(ProcessResponse::init(err.into()))
            }
        }
    }
}

// waiting for response
struct WaitingResponse<S> {
    stream: PipelineResponse,
    _s: PhantomData<S>,
}

impl<S> WaitingResponse<S> {

    #[inline]
    fn init(info: &mut PipelineInfo<S>, reply: Reply) -> PipelineState<S>
    {
        match reply.into() {
            ReplyItem::Message(resp) =>
                RunMiddlewares::init(info, resp),
            ReplyItem::Actor(ctx) =>
                PipelineState::Handler(
                    WaitingResponse { stream: PipelineResponse::Context(ctx), _s: PhantomData }),
            ReplyItem::Future(fut) =>
                PipelineState::Handler(
                    WaitingResponse { stream: PipelineResponse::Response(fut), _s: PhantomData }),
        }
    }

    fn poll(mut self, info: &mut PipelineInfo<S>) -> Result<PipelineState<S>, PipelineState<S>> {
        let stream = mem::replace(&mut self.stream, PipelineResponse::None);

        match stream {
            PipelineResponse::Context(mut context) => {
                loop {
                    match context.poll() {
                        Ok(Async::Ready(Some(frame))) => {
                            match frame {
                                Frame::Message(resp) => {
                                    info.context = Some(context);
                                    return Ok(RunMiddlewares::init(info, resp))
                                }
                                Frame::Payload(_) | Frame::Drain(_) => (),
                            }
                        },
                        Ok(Async::Ready(None)) => {
                            error!("Unexpected eof");
                            let err: Error = UnexpectedTaskFrame.into();
                            return Ok(ProcessResponse::init(err.into()))
                        },
                        Ok(Async::NotReady) => {
                            self.stream = PipelineResponse::Context(context);
                            return Err(PipelineState::Handler(self))
                        },
                        Err(err) =>
                            return Ok(ProcessResponse::init(err.into()))
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
                        Ok(RunMiddlewares::init(info, response)),
                    Err(err) =>
                        Ok(ProcessResponse::init(err.into())),
                }
            }
            PipelineResponse::None => {
                unreachable!("Broken internal state")
            }
        }

    }
}

/// Middlewares response executor
struct RunMiddlewares<S> {
    curr: usize,
    fut: Option<Box<Future<Item=HttpResponse, Error=Error>>>,
    _s: PhantomData<S>,
}

impl<S> RunMiddlewares<S> {

    fn init(info: &mut PipelineInfo<S>, mut resp: HttpResponse) -> PipelineState<S>
    {
        if info.count == 0 {
            return ProcessResponse::init(resp);
        }
        let mut curr = 0;
        let len = info.mws.len();

        loop {
            resp = match info.mws[curr].response(info.req_mut(), resp) {
                Response::Err(err) => {
                    info.count = curr + 1;
                    return ProcessResponse::init(err.into())
                }
                Response::Done(r) => {
                    curr += 1;
                    if curr == len {
                        return ProcessResponse::init(r)
                    } else {
                        r
                    }
                },
                Response::Future(fut) => {
                    return PipelineState::RunMiddlewares(
                        RunMiddlewares { curr: curr, fut: Some(fut), _s: PhantomData })
                },
            };
        }
    }

    fn poll(mut self, info: &mut PipelineInfo<S>) -> Result<PipelineState<S>, PipelineState<S>> {
        let len = info.mws.len();

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
                    return Ok(ProcessResponse::init(err.into())),
            };

            loop {
                if self.curr == len {
                    return Ok(ProcessResponse::init(resp));
                } else {
                    match info.mws[self.curr].response(info.req_mut(), resp) {
                        Response::Err(err) =>
                            return Ok(ProcessResponse::init(err.into())),
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

struct ProcessResponse<S> {
    resp: HttpResponse,
    iostate: IOState,
    running: RunningState,
    drain: Option<oneshot::Sender<()>>,
    _s: PhantomData<S>,
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

impl<S> ProcessResponse<S> {

    #[inline]
    fn init(resp: HttpResponse) -> PipelineState<S>
    {
        PipelineState::Response(
            ProcessResponse{ resp: resp,
                             iostate: IOState::Response,
                             running: RunningState::Running,
                             drain: None,
                             _s: PhantomData})
    }

    fn poll_io(mut self, io: &mut Writer, info: &mut PipelineInfo<S>)
               -> Result<PipelineState<S>, PipelineState<S>>
    {
        if self.drain.is_none() && self.running != RunningState::Paused {
            // if task is paused, write buffer is probably full

            loop {
                let result = match mem::replace(&mut self.iostate, IOState::Done) {
                    IOState::Response => {
                        let result = match io.start(info.req_mut().get_inner(),
                                                    &mut self.resp) {
                            Ok(res) => res,
                            Err(err) => {
                                info.error = Some(err.into());
                                return Ok(FinishingMiddlewares::init(info, self.resp))
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
                            match info.poll_context() {
                                Ok(Async::NotReady) => (),
                                Ok(Async::Ready(_)) =>
                                    self.running = RunningState::Done,
                                Err(err) => {
                                    info.error = Some(err);
                                    return Ok(FinishingMiddlewares::init(info, self.resp))
                                }
                            }
                        }

                        match body.poll() {
                            Ok(Async::Ready(None)) => {
                                self.iostate = IOState::Done;
                                if let Err(err) = io.write_eof() {
                                    info.error = Some(err.into());
                                    return Ok(FinishingMiddlewares::init(info, self.resp))
                                }
                                break
                            },
                            Ok(Async::Ready(Some(chunk))) => {
                                self.iostate = IOState::Payload(body);
                                match io.write(chunk.as_ref()) {
                                    Err(err) => {
                                        info.error = Some(err.into());
                                        return Ok(FinishingMiddlewares::init(info, self.resp))
                                    },
                                    Ok(result) => result
                                }
                            }
                            Ok(Async::NotReady) => {
                                self.iostate = IOState::Payload(body);
                                break
                            },
                            Err(err) => {
                                info.error = Some(err);
                                return Ok(FinishingMiddlewares::init(info, self.resp))
                            }
                        }
                    },
                    IOState::Context => {
                        match info.context.as_mut().unwrap().poll() {
                            Ok(Async::Ready(Some(frame))) => {
                                match frame {
                                    Frame::Message(msg) => {
                                        error!("Unexpected message frame {:?}", msg);
                                        info.error = Some(UnexpectedTaskFrame.into());
                                        return Ok(
                                            FinishingMiddlewares::init(info, self.resp))
                                    },
                                    Frame::Payload(None) => {
                                        self.iostate = IOState::Done;
                                        if let Err(err) = io.write_eof() {
                                            info.error = Some(err.into());
                                            return Ok(
                                                FinishingMiddlewares::init(info, self.resp))
                                        }
                                        break
                                    },
                                    Frame::Payload(Some(chunk)) => {
                                        self.iostate = IOState::Context;
                                        match io.write(chunk.as_ref()) {
                                            Err(err) => {
                                                info.error = Some(err.into());
                                                return Ok(FinishingMiddlewares::init(
                                                    info, self.resp))
                                            },
                                            Ok(result) => result
                                        }
                                    },
                                    Frame::Drain(fut) => {
                                        self.drain = Some(fut);
                                        break
                                    }
                                }
                            },
                            Ok(Async::Ready(None)) => {
                                self.iostate = IOState::Done;
                                info.context.take();
                                break
                            }
                            Ok(Async::NotReady) => {
                                self.iostate = IOState::Context;
                                break
                            }
                            Err(err) => {
                                info.error = Some(err);
                                return Ok(FinishingMiddlewares::init(info, self.resp))
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

        // flush io but only if we need to
        if self.running == RunningState::Paused || self.drain.is_some() {
            match io.poll_completed() {
                Ok(Async::Ready(_)) =>
                    self.running.resume(),
                Ok(Async::NotReady) =>
                    return Err(PipelineState::Response(self)),
                Err(err) => {
                    debug!("Error sending data: {}", err);
                    info.error = Some(err.into());
                    return Ok(FinishingMiddlewares::init(info, self.resp))
                }
            }
        }

        // drain futures
        if let Some(tx) = self.drain.take() {
            let _ = tx.send(());
        }

        // response is completed
        match self.iostate {
            IOState::Done => {
                self.resp.set_response_size(io.written());
                Ok(FinishingMiddlewares::init(info, self.resp))
            }
            _ => Err(PipelineState::Response(self))
        }
    }
}

/// Middlewares start executor
struct FinishingMiddlewares<S> {
    resp: HttpResponse,
    fut: Option<Box<Future<Item=(), Error=Error>>>,
    _s: PhantomData<S>,
}

impl<S> FinishingMiddlewares<S> {

    fn init(info: &mut PipelineInfo<S>, resp: HttpResponse) -> PipelineState<S> {
        if info.count == 0 {
            Completed::init(info)
        } else {
            match (FinishingMiddlewares{resp: resp, fut: None, _s: PhantomData}).poll(info) {
                Ok(st) | Err(st) => st,
            }
        }
    }

    fn poll(mut self, info: &mut PipelineInfo<S>) -> Result<PipelineState<S>, PipelineState<S>> {
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
            info.count -= 1;

            match info.mws[info.count].finish(info.req_mut(), &self.resp) {
                Finished::Done => {
                    if info.count == 0 {
                        return Ok(Completed::init(info))
                    }
                }
                Finished::Future(fut) => {
                    self.fut = Some(fut);
                },
            }
        }
    }
}

struct Completed<S>(PhantomData<S>);

impl<S> Completed<S> {

    #[inline]
    fn init(info: &mut PipelineInfo<S>) -> PipelineState<S> {
        if info.context.is_none() {
            PipelineState::None
        } else {
            PipelineState::Completed(Completed(PhantomData))
        }
    }

    #[inline]
    fn poll(self, info: &mut PipelineInfo<S>) -> Result<PipelineState<S>, PipelineState<S>> {
        match info.poll_context() {
            Ok(Async::NotReady) => Ok(PipelineState::Completed(Completed(PhantomData))),
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

    impl<S> PipelineState<S> {
        fn is_none(&self) -> Option<bool> {
            if let PipelineState::None = *self { Some(true) } else { None }
        }
        fn completed(self) -> Option<Completed<S>> {
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
            let mut info = PipelineInfo::new(HttpRequest::default());
            Completed::init(&mut info).is_none().unwrap();

            let req = HttpRequest::default();
            let mut ctx = HttpContext::new(req.clone(), MyActor);
            let addr: Address<_> = ctx.address();
            let mut info = PipelineInfo::new(req);
            info.context = Some(Box::new(ctx));
            let mut state = Completed::init(&mut info).completed().unwrap();

            let st = state.poll(&mut info).ok().unwrap();
            let pp = Pipeline(info, st);
            assert!(!pp.is_done());

            let Pipeline(mut info, st) = pp;
            state = st.completed().unwrap();
            drop(addr);

            state.poll(&mut info).ok().unwrap().is_none().unwrap();

            result(Ok::<_, ()>(()))
        })).unwrap()
    }
}
