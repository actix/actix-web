use std::marker::PhantomData;
use std::rc::Rc;
use std::{io, mem};

use futures::sync::oneshot;
use futures::{Async, Future, Poll, Stream};
use log::Level::Debug;

use body::{Body, BodyStream};
use context::{ActorHttpContext, Frame};
use error::Error;
use handler::{AsyncResult, AsyncResultItem};
use header::ContentEncoding;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use middleware::{Finished, Middleware, Response, Started};
use server::{HttpHandlerTask, Writer, WriterState};

#[doc(hidden)]
pub trait PipelineHandler<S> {
    fn encoding(&self) -> ContentEncoding;

    fn handle(&self, &HttpRequest<S>) -> AsyncResult<HttpResponse>;
}

#[doc(hidden)]
pub struct Pipeline<S: 'static, H>(
    PipelineInfo<S>,
    PipelineState<S, H>,
    Rc<Vec<Box<Middleware<S>>>>,
);

enum PipelineState<S, H> {
    None,
    Error,
    Starting(StartMiddlewares<S, H>),
    Handler(WaitingResponse<S, H>),
    RunMiddlewares(RunMiddlewares<S, H>),
    Response(ProcessResponse<S, H>),
    Finishing(FinishingMiddlewares<S, H>),
    Completed(Completed<S, H>),
}

impl<S: 'static, H: PipelineHandler<S>> PipelineState<S, H> {
    fn poll(
        &mut self, info: &mut PipelineInfo<S>, mws: &[Box<Middleware<S>>],
    ) -> Option<PipelineState<S, H>> {
        match *self {
            PipelineState::Starting(ref mut state) => state.poll(info, mws),
            PipelineState::Handler(ref mut state) => state.poll(info, mws),
            PipelineState::RunMiddlewares(ref mut state) => state.poll(info, mws),
            PipelineState::Finishing(ref mut state) => state.poll(info, mws),
            PipelineState::Completed(ref mut state) => state.poll(info),
            PipelineState::Response(ref mut state) => state.poll(info, mws),
            PipelineState::None | PipelineState::Error => {
                None
            }
        }
    }
}

struct PipelineInfo<S: 'static> {
    req: HttpRequest<S>,
    count: u16,
    context: Option<Box<ActorHttpContext>>,
    error: Option<Error>,
    disconnected: Option<bool>,
    encoding: ContentEncoding,
}

impl<S: 'static> PipelineInfo<S> {
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

impl<S: 'static, H: PipelineHandler<S>> Pipeline<S, H> {
    pub fn new(
        req: HttpRequest<S>, mws: Rc<Vec<Box<Middleware<S>>>>, handler: Rc<H>,
    ) -> Pipeline<S, H> {
        let mut info = PipelineInfo {
            req,
            count: 0,
            error: None,
            context: None,
            disconnected: None,
            encoding: handler.encoding(),
        };
        let state = StartMiddlewares::init(&mut info, &mws, handler);

        Pipeline(info, state, mws)
    }
}

impl<S: 'static, H> Pipeline<S, H> {
    #[inline]
    fn is_done(&self) -> bool {
        match self.1 {
            PipelineState::None
            | PipelineState::Error
            | PipelineState::Starting(_)
            | PipelineState::Handler(_)
            | PipelineState::RunMiddlewares(_)
            | PipelineState::Response(_) => true,
            PipelineState::Finishing(_) | PipelineState::Completed(_) => false,
        }
    }
}

impl<S: 'static, H: PipelineHandler<S>> HttpHandlerTask for Pipeline<S, H> {
    fn disconnected(&mut self) {
        self.0.disconnected = Some(true);
    }

    fn poll_io(&mut self, io: &mut Writer) -> Poll<bool, Error> {
        let mut state = mem::replace(&mut self.1, PipelineState::None);

        loop {
            if let PipelineState::Response(st) = state {
                match st.poll_io(io, &mut self.0, &self.2) {
                    Ok(state) => {
                        self.1 = state;
                        if let Some(error) = self.0.error.take() {
                            return Err(error);
                        } else {
                            return Ok(Async::Ready(self.is_done()));
                        }
                    }
                    Err(state) => {
                        self.1 = state;
                        return Ok(Async::NotReady);
                    }
                }
            }
            match state {
                PipelineState::None => return Ok(Async::Ready(true)),
                PipelineState::Error => {
                    return Err(
                        io::Error::new(io::ErrorKind::Other, "Internal error").into()
                    )
                }
                _ => (),
            }

            match state.poll(&mut self.0, &self.2) {
                Some(st) => state = st,
                None => {
                    return {
                        self.1 = state;
                        Ok(Async::NotReady)
                    }
                }
            }
        }
    }

    fn poll_completed(&mut self) -> Poll<(), Error> {
        let mut state = mem::replace(&mut self.1, PipelineState::None);
        loop {
            match state {
                PipelineState::None | PipelineState::Error => {
                    return Ok(Async::Ready(()))
                }
                _ => (),
            }

            if let Some(st) = state.poll(&mut self.0, &self.2) {
                state = st;
            } else {
                self.1 = state;
                return Ok(Async::NotReady);
            }
        }
    }
}

type Fut = Box<Future<Item = Option<HttpResponse>, Error = Error>>;

/// Middlewares start executor
struct StartMiddlewares<S, H> {
    hnd: Rc<H>,
    fut: Option<Fut>,
    _s: PhantomData<S>,
}

impl<S: 'static, H: PipelineHandler<S>> StartMiddlewares<S, H> {
    fn init(
        info: &mut PipelineInfo<S>, mws: &[Box<Middleware<S>>], hnd: Rc<H>,
    ) -> PipelineState<S, H> {
        // execute middlewares, we need this stage because middlewares could be
        // non-async and we can move to next state immediately
        let len = mws.len() as u16;

        loop {
            if info.count == len {
                let reply = hnd.handle(&info.req);
                return WaitingResponse::init(info, mws, reply);
            } else {
                match mws[info.count as usize].start(&info.req) {
                    Ok(Started::Done) => info.count += 1,
                    Ok(Started::Response(resp)) => {
                        return RunMiddlewares::init(info, mws, resp);
                    }
                    Ok(Started::Future(fut)) => {
                        return PipelineState::Starting(StartMiddlewares {
                            hnd,
                            fut: Some(fut),
                            _s: PhantomData,
                        })
                    }
                    Err(err) => {
                        return RunMiddlewares::init(info, mws, err.into());
                    }
                }
            }
        }
    }

    fn poll(
        &mut self, info: &mut PipelineInfo<S>, mws: &[Box<Middleware<S>>],
    ) -> Option<PipelineState<S, H>> {
        let len = mws.len() as u16;

        'outer: loop {
            match self.fut.as_mut().unwrap().poll() {
                Ok(Async::NotReady) => {
                    return None;
                }
                Ok(Async::Ready(resp)) => {
                    info.count += 1;
                    if let Some(resp) = resp {
                        return Some(RunMiddlewares::init(info, mws, resp));
                    }
                    loop {
                        if info.count == len {
                            let reply = self.hnd.handle(&info.req);
                            return Some(WaitingResponse::init(info, mws, reply));
                        } else {
                            let res = mws[info.count as usize].start(&info.req);
                            match res {
                                Ok(Started::Done) => info.count += 1,
                                Ok(Started::Response(resp)) => {
                                    return Some(RunMiddlewares::init(info, mws, resp));
                                }
                                Ok(Started::Future(fut)) => {
                                    self.fut = Some(fut);
                                    continue 'outer;
                                }
                                Err(err) => {
                                    return Some(RunMiddlewares::init(
                                        info,
                                        mws,
                                        err.into(),
                                    ));
                                }
                            }
                        }
                    }
                }
                Err(err) => {
                    return Some(RunMiddlewares::init(info, mws, err.into()));
                }
            }
        }
    }
}

// waiting for response
struct WaitingResponse<S, H> {
    fut: Box<Future<Item = HttpResponse, Error = Error>>,
    _s: PhantomData<S>,
    _h: PhantomData<H>,
}

impl<S: 'static, H> WaitingResponse<S, H> {
    #[inline]
    fn init(
        info: &mut PipelineInfo<S>, mws: &[Box<Middleware<S>>],
        reply: AsyncResult<HttpResponse>,
    ) -> PipelineState<S, H> {
        match reply.into() {
            AsyncResultItem::Ok(resp) => RunMiddlewares::init(info, mws, resp),
            AsyncResultItem::Err(err) => RunMiddlewares::init(info, mws, err.into()),
            AsyncResultItem::Future(fut) => PipelineState::Handler(WaitingResponse {
                fut,
                _s: PhantomData,
                _h: PhantomData,
            }),
        }
    }

    fn poll(
        &mut self, info: &mut PipelineInfo<S>, mws: &[Box<Middleware<S>>],
    ) -> Option<PipelineState<S, H>> {
        match self.fut.poll() {
            Ok(Async::NotReady) => None,
            Ok(Async::Ready(resp)) => Some(RunMiddlewares::init(info, mws, resp)),
            Err(err) => Some(RunMiddlewares::init(info, mws, err.into())),
        }
    }
}

/// Middlewares response executor
struct RunMiddlewares<S, H> {
    curr: usize,
    fut: Option<Box<Future<Item = HttpResponse, Error = Error>>>,
    _s: PhantomData<S>,
    _h: PhantomData<H>,
}

impl<S: 'static, H> RunMiddlewares<S, H> {
    #[inline]
    fn init(
        info: &mut PipelineInfo<S>, mws: &[Box<Middleware<S>>], mut resp: HttpResponse,
    ) -> PipelineState<S, H> {
        if info.count == 0 {
            return ProcessResponse::init(resp);
        }
        let mut curr = 0;
        let len = mws.len();

        loop {
            let state = mws[curr].response(&info.req, resp);
            resp = match state {
                Err(err) => {
                    info.count = (curr + 1) as u16;
                    return ProcessResponse::init(err.into());
                }
                Ok(Response::Done(r)) => {
                    curr += 1;
                    if curr == len {
                        return ProcessResponse::init(r);
                    } else {
                        r
                    }
                }
                Ok(Response::Future(fut)) => {
                    return PipelineState::RunMiddlewares(RunMiddlewares {
                        curr,
                        fut: Some(fut),
                        _s: PhantomData,
                        _h: PhantomData,
                    });
                }
            };
        }
    }

    fn poll(
        &mut self, info: &mut PipelineInfo<S>, mws: &[Box<Middleware<S>>],
    ) -> Option<PipelineState<S, H>> {
        let len = mws.len();

        loop {
            // poll latest fut
            let mut resp = match self.fut.as_mut().unwrap().poll() {
                Ok(Async::NotReady) => return None,
                Ok(Async::Ready(resp)) => {
                    self.curr += 1;
                    resp
                }
                Err(err) => return Some(ProcessResponse::init(err.into())),
            };

            loop {
                if self.curr == len {
                    return Some(ProcessResponse::init(resp));
                } else {
                    let state = mws[self.curr].response(&info.req, resp);
                    match state {
                        Err(err) => return Some(ProcessResponse::init(err.into())),
                        Ok(Response::Done(r)) => {
                            self.curr += 1;
                            resp = r
                        }
                        Ok(Response::Future(fut)) => {
                            self.fut = Some(fut);
                            break;
                        }
                    }
                }
            }
        }
    }
}

struct ProcessResponse<S, H> {
    resp: Option<HttpResponse>,
    iostate: IOState,
    running: RunningState,
    drain: Option<oneshot::Sender<()>>,
    _s: PhantomData<S>,
    _h: PhantomData<H>,
}

#[derive(PartialEq, Debug)]
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
    Actor(Box<ActorHttpContext>),
    Done,
}

impl<S: 'static, H> ProcessResponse<S, H> {
    #[inline]
    fn init(resp: HttpResponse) -> PipelineState<S, H> {
        PipelineState::Response(ProcessResponse {
            resp: Some(resp),
            iostate: IOState::Response,
            running: RunningState::Running,
            drain: None,
            _s: PhantomData,
            _h: PhantomData,
        })
    }

    fn poll(
        &mut self, info: &mut PipelineInfo<S>, mws: &[Box<Middleware<S>>],
    ) -> Option<PipelineState<S, H>> {
        // connection is dead at this point
        match mem::replace(&mut self.iostate, IOState::Done) {
            IOState::Response =>
                Some(FinishingMiddlewares::init(info, mws, self.resp.take().unwrap())),
            IOState::Payload(_) =>
                Some(FinishingMiddlewares::init(info, mws, self.resp.take().unwrap())),
            IOState::Actor(mut ctx) => {
                if info.disconnected.take().is_some() {
                    ctx.disconnected();
                }
                loop {
                    match ctx.poll() {
                        Ok(Async::Ready(Some(vec))) => {
                            if vec.is_empty() {
                                continue;
                            }
                            for frame in vec {
                                match frame {
                                    Frame::Chunk(None) => {
                                        info.context = Some(ctx);
                                        return Some(FinishingMiddlewares::init(
                                            info, mws, self.resp.take().unwrap(),
                                        ))
                                    }
                                    Frame::Chunk(Some(_)) => (),
                                    Frame::Drain(fut) => {let _ = fut.send(());},
                                }
                            }
                        }
                        Ok(Async::Ready(None)) => 
                            return Some(FinishingMiddlewares::init(
                                info, mws, self.resp.take().unwrap(),
                            )),
                        Ok(Async::NotReady) => {
                            self.iostate = IOState::Actor(ctx);
                            return None;
                        }
                        Err(err) => {
                            info.context = Some(ctx);
                            info.error = Some(err);
                            return Some(FinishingMiddlewares::init(info, mws, self.resp.take().unwrap()));
                        }
                    }
                }
            }
            IOState::Done => Some(FinishingMiddlewares::init(info, mws, self.resp.take().unwrap()))
        }
    }

    fn poll_io(
        mut self, io: &mut Writer, info: &mut PipelineInfo<S>,
        mws: &[Box<Middleware<S>>],
    ) -> Result<PipelineState<S, H>, PipelineState<S, H>> {
        loop {
            if self.drain.is_none() && self.running != RunningState::Paused {
                // if task is paused, write buffer is probably full
                'inner: loop {
                    let result = match mem::replace(&mut self.iostate, IOState::Done) {
                        IOState::Response => {
                            let encoding =
                                self.resp.as_ref().unwrap().content_encoding().unwrap_or(info.encoding);

                            let result =
                                match io.start(&info.req, self.resp.as_mut().unwrap(), encoding) {
                                    Ok(res) => res,
                                    Err(err) => {
                                        info.error = Some(err.into());
                                        return Ok(FinishingMiddlewares::init(
                                            info, mws, self.resp.take().unwrap(),
                                        ));
                                    }
                                };

                            if let Some(err) = self.resp.as_ref().unwrap().error() {
                                if self.resp.as_ref().unwrap().status().is_server_error() {
                                    error!(
                                        "Error occured during request handling, status: {} {}",
                                        self.resp.as_ref().unwrap().status(), err
                                    );
                                } else {
                                    warn!(
                                        "Error occured during request handling: {}",
                                        err
                                    );
                                }
                                if log_enabled!(Debug) {
                                    debug!("{:?}", err);
                                }
                            }

                            // always poll stream or actor for the first time
                            match self.resp.as_mut().unwrap().replace_body(Body::Empty) {
                                Body::Streaming(stream) => {
                                    self.iostate = IOState::Payload(stream);
                                    continue 'inner;
                                }
                                Body::Actor(ctx) => {
                                    self.iostate = IOState::Actor(ctx);
                                    continue 'inner;
                                }
                                _ => (),
                            }

                            result
                        }
                        IOState::Payload(mut body) => match body.poll() {
                            Ok(Async::Ready(None)) => {
                                if let Err(err) = io.write_eof() {
                                    info.error = Some(err.into());
                                    return Ok(FinishingMiddlewares::init(
                                        info, mws, self.resp.take().unwrap(),
                                    ));
                                }
                                break;
                            }
                            Ok(Async::Ready(Some(chunk))) => {
                                self.iostate = IOState::Payload(body);
                                match io.write(&chunk.into()) {
                                    Err(err) => {
                                        info.error = Some(err.into());
                                        return Ok(FinishingMiddlewares::init(
                                            info, mws, self.resp.take().unwrap(),
                                        ));
                                    }
                                    Ok(result) => result,
                                }
                            }
                            Ok(Async::NotReady) => {
                                self.iostate = IOState::Payload(body);
                                break;
                            }
                            Err(err) => {
                                info.error = Some(err);
                                return Ok(FinishingMiddlewares::init(
                                    info, mws, self.resp.take().unwrap(),
                                ));
                            }
                        },
                        IOState::Actor(mut ctx) => {
                            if info.disconnected.take().is_some() {
                                ctx.disconnected();
                            }
                            match ctx.poll() {
                                Ok(Async::Ready(Some(vec))) => {
                                    if vec.is_empty() {
                                        self.iostate = IOState::Actor(ctx);
                                        break;
                                    }
                                    let mut res = None;
                                    for frame in vec {
                                        match frame {
                                            Frame::Chunk(None) => {
                                                info.context = Some(ctx);
                                                if let Err(err) = io.write_eof() {
                                                    info.error = Some(err.into());
                                                    return Ok(
                                                        FinishingMiddlewares::init(
                                                            info, mws, self.resp.take().unwrap(),
                                                        ),
                                                    );
                                                }
                                                break 'inner;
                                            }
                                            Frame::Chunk(Some(chunk)) => {
                                                match io.write(&chunk) {
                                                    Err(err) => {
                                                        info.context = Some(ctx);
                                                        info.error = Some(err.into());
                                                        return Ok(
                                                            FinishingMiddlewares::init(
                                                                info, mws, self.resp.take().unwrap(),
                                                            ),
                                                        );
                                                    }
                                                    Ok(result) => res = Some(result),
                                                }
                                            }
                                            Frame::Drain(fut) => self.drain = Some(fut),
                                        }
                                    }
                                    self.iostate = IOState::Actor(ctx);
                                    if self.drain.is_some() {
                                        self.running.resume();
                                        break 'inner;
                                    }
                                    res.unwrap()
                                }
                                Ok(Async::Ready(None)) => break,
                                Ok(Async::NotReady) => {
                                    self.iostate = IOState::Actor(ctx);
                                    break;
                                }
                                Err(err) => {
                                    info.context = Some(ctx);
                                    info.error = Some(err);
                                    return Ok(FinishingMiddlewares::init(
                                        info, mws, self.resp.take().unwrap(),
                                    ));
                                }
                            }
                        }
                        IOState::Done => break,
                    };

                    match result {
                        WriterState::Pause => {
                            self.running.pause();
                            break;
                        }
                        WriterState::Done => self.running.resume(),
                    }
                }
            }

            // flush io but only if we need to
            if self.running == RunningState::Paused || self.drain.is_some() {
                match io.poll_completed(false) {
                    Ok(Async::Ready(_)) => {
                        self.running.resume();

                        // resolve drain futures
                        if let Some(tx) = self.drain.take() {
                            let _ = tx.send(());
                        }
                        // restart io processing
                        continue;
                    }
                    Ok(Async::NotReady) => return Err(PipelineState::Response(self)),
                    Err(err) => {
                        if let IOState::Actor(mut ctx) =
                            mem::replace(&mut self.iostate, IOState::Done)
                        {
                            ctx.disconnected();
                            info.context = Some(ctx);
                        }
                        info.error = Some(err.into());
                        return Ok(FinishingMiddlewares::init(info, mws, self.resp.take().unwrap()));
                    }
                }
            }
            break;
        }

        // response is completed
        match self.iostate {
            IOState::Done => {
                match io.write_eof() {
                    Ok(_) => (),
                    Err(err) => {
                        info.error = Some(err.into());
                        return Ok(FinishingMiddlewares::init(info, mws, self.resp.take().unwrap()));
                    }
                }
                self.resp.as_mut().unwrap().set_response_size(io.written());
                Ok(FinishingMiddlewares::init(info, mws, self.resp.take().unwrap()))
            }
            _ => Err(PipelineState::Response(self)),
        }
    }
}

/// Middlewares start executor
struct FinishingMiddlewares<S, H> {
    resp: Option<HttpResponse>,
    fut: Option<Box<Future<Item = (), Error = Error>>>,
    _s: PhantomData<S>,
    _h: PhantomData<H>,
}

impl<S: 'static, H> FinishingMiddlewares<S, H> {
    #[inline]
    fn init(
        info: &mut PipelineInfo<S>, mws: &[Box<Middleware<S>>], resp: HttpResponse,
    ) -> PipelineState<S, H> {
        if info.count == 0 {
            resp.release();
            Completed::init(info)
        } else {
            let mut state = FinishingMiddlewares {
                resp: Some(resp),
                fut: None,
                _s: PhantomData,
                _h: PhantomData,
            };
            if let Some(st) = state.poll(info, mws) {
                st
            } else {
                PipelineState::Finishing(state)
            }
        }
    }

    fn poll(
        &mut self, info: &mut PipelineInfo<S>, mws: &[Box<Middleware<S>>],
    ) -> Option<PipelineState<S, H>> {
        loop {
            // poll latest fut
            let not_ready = if let Some(ref mut fut) = self.fut {
                match fut.poll() {
                    Ok(Async::NotReady) => true,
                    Ok(Async::Ready(())) => false,
                    Err(err) => {
                        error!("Middleware finish error: {}", err);
                        false
                    }
                }
            } else {
                false
            };
            if not_ready {
                return None;
            }
            self.fut = None;
            if info.count == 0 {
                self.resp.take().unwrap().release();
                return Some(Completed::init(info));
            }

            info.count -= 1;
            let state =
                mws[info.count as usize].finish(&info.req, self.resp.as_ref().unwrap());
            match state {
                Finished::Done => {
                    if info.count == 0 {
                        self.resp.take().unwrap().release();
                        return Some(Completed::init(info));
                    }
                }
                Finished::Future(fut) => {
                    self.fut = Some(fut);
                }
            }
        }
    }
}

#[derive(Debug)]
struct Completed<S, H>(PhantomData<S>, PhantomData<H>);

impl<S, H> Completed<S, H> {
    #[inline]
    fn init(info: &mut PipelineInfo<S>) -> PipelineState<S, H> {
        if let Some(ref err) = info.error {
            error!("Error occurred during request handling: {}", err);
        }

        if info.context.is_none() {
            PipelineState::None
        } else {
            match info.poll_context() {
                Ok(Async::NotReady) => {
                    PipelineState::Completed(Completed(PhantomData, PhantomData))
                }
                Ok(Async::Ready(())) => PipelineState::None,
                Err(_) => PipelineState::Error,
            }
        }
    }

    #[inline]
    fn poll(&mut self, info: &mut PipelineInfo<S>) -> Option<PipelineState<S, H>> {
        match info.poll_context() {
            Ok(Async::NotReady) => None,
            Ok(Async::Ready(())) => Some(PipelineState::None),
            Err(_) => Some(PipelineState::Error),
        }
    }
}
