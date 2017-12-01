use std::{fmt, mem};
use std::rc::Rc;
use std::cell::RefCell;

use futures::{Async, Future, Poll};
use futures::task::{Task as FutureTask, current as current_task};

use route::{Reply, ReplyItem};
use body::{Body, BodyStream, Binary};
use context::Frame;
use h1writer::{Writer, WriterState};
use error::{Error, UnexpectedTaskFrame};
use pipeline::MiddlewaresResponse;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

#[derive(PartialEq, Debug)]
enum TaskRunningState {
    Paused,
    Running,
    Done,
}

impl TaskRunningState {
    fn is_done(&self) -> bool {
        *self == TaskRunningState::Done
    }
    fn pause(&mut self) {
        if *self != TaskRunningState::Done {
            *self = TaskRunningState::Paused
        }
    }
    fn resume(&mut self) {
        if *self != TaskRunningState::Done {
            *self = TaskRunningState::Running
        }
    }
}

enum ResponseState {
    Reading,
    Ready(HttpResponse),
    Middlewares(MiddlewaresResponse),
    Prepared(Option<HttpResponse>),
}

enum IOState {
    Response,
    Payload(BodyStream),
    Context,
    Done,
}

enum TaskStream {
    None,
    Context(Box<IoContext>),
    Response(Box<Future<Item=HttpResponse, Error=Error>>),
}

impl IOState {
    fn is_done(&self) -> bool {
        match *self {
            IOState::Done => true,
            _ => false
        }
    }
}

impl ResponseState {
    fn is_reading(&self) -> bool {
        match *self {
            ResponseState::Reading => true,
            _ => false
        }
    }
}

impl fmt::Debug for ResponseState {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ResponseState::Reading => write!(f, "ResponseState::Reading"),
            ResponseState::Ready(_) => write!(f, "ResponseState::Ready"),
            ResponseState::Middlewares(_) => write!(f, "ResponseState::Middlewares"),
            ResponseState::Prepared(_) => write!(f, "ResponseState::Prepared"),
        }
    }
}

impl fmt::Debug for IOState {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            IOState::Response => write!(f, "IOState::Response"),
            IOState::Payload(_) => write!(f, "IOState::Payload"),
            IOState::Context => write!(f, "IOState::Context"),
            IOState::Done => write!(f, "IOState::Done"),
        }
    }
}

pub(crate) trait IoContext: 'static {
    fn disconnected(&mut self);
    fn poll(&mut self) -> Poll<Option<Frame>, Error>;
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

pub(crate) struct Task {
    running: TaskRunningState,
    response: ResponseState,
    iostate: IOState,
    stream: TaskStream,
    drain: Vec<Rc<RefCell<DrainFut>>>,
    middlewares: Option<MiddlewaresResponse>,
}

impl Task {

    pub(crate) fn new(reply: Reply) -> Task {
        match reply.into() {
            ReplyItem::Message(msg) => {
                Task::from_response(msg)
            },
            ReplyItem::Actor(ctx) => {
                Task { running: TaskRunningState::Running,
                       response: ResponseState::Reading,
                       iostate: IOState::Response,
                       drain: Vec::new(),
                       stream: TaskStream::Context(ctx),
                       middlewares: None }
            }
            ReplyItem::Future(fut) => {
                Task { running: TaskRunningState::Running,
                       response: ResponseState::Reading,
                       iostate: IOState::Response,
                       drain: Vec::new(),
                       stream: TaskStream::Response(fut),
                       middlewares: None }
            }
        }
    }

    pub(crate) fn from_response<R: Into<HttpResponse>>(response: R) -> Task {
        Task { running: TaskRunningState::Running,
               response: ResponseState::Ready(response.into()),
               iostate: IOState::Response,
               drain: Vec::new(),
               stream: TaskStream::None,
               middlewares: None }
    }

    pub(crate) fn from_error<E: Into<Error>>(err: E) -> Task {
        Task::from_response(err.into())
    }

    pub(crate) fn response(&mut self) -> HttpResponse {
        match self.response {
            ResponseState::Prepared(ref mut state) => state.take().unwrap(),
            _ => panic!("Internal state is broken"),
        }
    }

    pub(crate) fn set_middlewares(&mut self, middlewares: MiddlewaresResponse) {
        self.middlewares = Some(middlewares)
    }

    pub(crate) fn disconnected(&mut self) {
        if let TaskStream::Context(ref mut ctx) = self.stream {
            ctx.disconnected();
        }
    }

    pub(crate) fn poll_io<T>(&mut self, io: &mut T, req: &mut HttpRequest) -> Poll<bool, Error>
        where T: Writer
    {
        trace!("POLL-IO frames resp: {:?}, io: {:?}, running: {:?}",
               self.response, self.iostate, self.running);

        if self.iostate.is_done() {  // response is completed
            return Ok(Async::Ready(self.running.is_done()));
        } else if self.drain.is_empty() && self.running != TaskRunningState::Paused {
            // if task is paused, write buffer is probably full

            loop {
                let result = match mem::replace(&mut self.iostate, IOState::Done) {
                    IOState::Response => {
                        match self.poll_response(req) {
                            Ok(Async::Ready(mut resp)) => {
                                let result = io.start(req, &mut resp)?;

                                match resp.replace_body(Body::Empty) {
                                    Body::Streaming(stream) | Body::Upgrade(stream) =>
                                        self.iostate = IOState::Payload(stream),
                                    Body::StreamingContext | Body::UpgradeContext =>
                                        self.iostate = IOState::Context,
                                    _ => (),
                                }
                                self.response = ResponseState::Prepared(Some(resp));
                                result
                            },
                            Ok(Async::NotReady) => {
                                self.iostate = IOState::Response;
                                return Ok(Async::NotReady)
                            }
                            Err(err) => {
                                let mut resp = err.into();
                                let result = io.start(req, &mut resp)?;

                                match resp.replace_body(Body::Empty) {
                                    Body::Streaming(stream) | Body::Upgrade(stream) =>
                                        self.iostate = IOState::Payload(stream),
                                    _ => (),
                                }
                                self.response = ResponseState::Prepared(Some(resp));
                                result
                            }
                        }
                    },
                    IOState::Payload(mut body) => {
                        // always poll stream
                        if self.running == TaskRunningState::Running {
                            match self.poll()? {
                                Async::Ready(_) =>
                                    self.running = TaskRunningState::Done,
                                Async::NotReady => (),
                            }
                        }

                        match body.poll() {
                            Ok(Async::Ready(None)) => {
                                self.iostate = IOState::Done;
                                io.write_eof()?;
                                break
                            },
                            Ok(Async::Ready(Some(chunk))) => {
                                self.iostate = IOState::Payload(body);
                                io.write(chunk.as_ref())?
                            }
                            Ok(Async::NotReady) => {
                                self.iostate = IOState::Payload(body);
                                break
                            },
                            Err(err) => return Err(err),
                        }
                    }
                    IOState::Context => {
                        match self.poll_context() {
                            Ok(Async::Ready(None)) => {
                                self.iostate = IOState::Done;
                                self.running = TaskRunningState::Done;
                                io.write_eof()?;
                                break
                            },
                            Ok(Async::Ready(Some(chunk))) => {
                                self.iostate = IOState::Context;
                                io.write(chunk.as_ref())?
                            }
                            Ok(Async::NotReady) => {
                                self.iostate = IOState::Context;
                                break
                            }
                            Err(err) => return Err(err),
                        }
                    }
                    IOState::Done => break,
                };

                match result {
                    WriterState::Pause => {
                        self.running.pause();
                        break
                    }
                    WriterState::Done =>
                        self.running.resume(),
                }
            }
        }

        // flush io
        match io.poll_complete() {
            Ok(Async::Ready(_)) => self.running.resume(),
            Ok(Async::NotReady) => return Ok(Async::NotReady),
            Err(err) => {
                debug!("Error sending data: {}", err);
                return Err(err.into())
            }
        }

        // drain futures
        if !self.drain.is_empty() {
            for fut in &mut self.drain {
                fut.borrow_mut().set()
            }
            self.drain.clear();
        }

        // response is completed
        if self.iostate.is_done() {
            if let ResponseState::Prepared(Some(ref mut resp)) = self.response {
                resp.set_response_size(io.written())
            }
            Ok(Async::Ready(self.running.is_done()))
        } else {
            Ok(Async::NotReady)
        }
    }

    pub(crate) fn poll_response(&mut self, req: &mut HttpRequest) -> Poll<HttpResponse, Error> {
        loop {
            let state = mem::replace(&mut self.response, ResponseState::Prepared(None));
            match state {
                ResponseState::Ready(response) => {
                    // run middlewares
                    if let Some(mut middlewares) = self.middlewares.take() {
                        match middlewares.response(req, response) {
                            Ok(Some(response)) =>
                                return Ok(Async::Ready(response)),
                            Ok(None) => {
                                // middlewares need to run some futures
                                self.response = ResponseState::Middlewares(middlewares);
                                continue
                            }
                            Err(err) => return Err(err),
                        }
                    } else {
                        return Ok(Async::Ready(response))
                    }
                }
                ResponseState::Middlewares(mut middlewares) => {
                    // process middlewares
                    match middlewares.poll(req) {
                        Ok(Async::NotReady) => {
                            self.response = ResponseState::Middlewares(middlewares);
                            return Ok(Async::NotReady)
                        },
                        Ok(Async::Ready(response)) =>
                            return Ok(Async::Ready(response)),
                        Err(err) =>
                            return Err(err),
                    }
                }
                _ => (),
            }
            self.response = state;

            match mem::replace(&mut self.stream, TaskStream::None) {
                TaskStream::None =>
                    return Ok(Async::NotReady),
                TaskStream::Context(mut context) => {
                    loop {
                        match context.poll() {
                            Ok(Async::Ready(Some(frame))) => {
                                match frame {
                                    Frame::Message(msg) => {
                                        if !self.response.is_reading() {
                                            error!("Unexpected message frame {:?}", msg);
                                            return Err(UnexpectedTaskFrame.into())
                                        }
                                        self.stream = TaskStream::Context(context);
                                        self.response = ResponseState::Ready(msg);
                                        break
                                    },
                                    Frame::Payload(_) | Frame::Drain(_) => (),
                                }
                            },
                            Ok(Async::Ready(None)) => {
                                error!("Unexpected eof");
                                return Err(UnexpectedTaskFrame.into())
                            },
                            Ok(Async::NotReady) => {
                                self.stream = TaskStream::Context(context);
                                return Ok(Async::NotReady)
                            },
                            Err(err) =>
                                return Err(err),
                        }
                    }
                },
                TaskStream::Response(mut fut) => {
                    match fut.poll() {
                        Ok(Async::NotReady) => {
                            self.stream = TaskStream::Response(fut);
                            return Ok(Async::NotReady);
                        },
                        Ok(Async::Ready(response)) => {
                            self.response = ResponseState::Ready(response);
                        }
                        Err(err) =>
                            return Err(err)
                    }
                }
            }
        }
    }

    pub(crate) fn poll(&mut self) -> Poll<(), Error> {
        match self.stream {
            TaskStream::None | TaskStream::Response(_) =>
                Ok(Async::Ready(())),
            TaskStream::Context(ref mut context) => {
                loop {
                    match context.poll() {
                        Ok(Async::Ready(Some(_))) => (),
                        Ok(Async::Ready(None)) =>
                            return Ok(Async::Ready(())),
                        Ok(Async::NotReady) =>
                            return Ok(Async::NotReady),
                        Err(err) =>
                            return Err(err),
                    }
                }
            },
        }
    }

    fn poll_context(&mut self) -> Poll<Option<Binary>, Error> {
        match self.stream {
            TaskStream::None | TaskStream::Response(_) =>
                Err(UnexpectedTaskFrame.into()),
            TaskStream::Context(ref mut context) => {
                match context.poll() {
                    Ok(Async::Ready(Some(frame))) => {
                        match frame {
                            Frame::Message(msg) => {
                                error!("Unexpected message frame {:?}", msg);
                                Err(UnexpectedTaskFrame.into())
                            },
                            Frame::Payload(payload) => {
                                Ok(Async::Ready(payload))
                            },
                            Frame::Drain(fut) => {
                                self.drain.push(fut);
                                Ok(Async::NotReady)
                            }
                        }
                    },
                    Ok(Async::Ready(None)) => Ok(Async::Ready(None)),
                    Ok(Async::NotReady) => Ok(Async::NotReady),
                    Err(err) => Err(err),
                }
            },
        }
    }
}
