use std::mem;
use std::rc::Rc;
use std::cell::RefCell;
use std::collections::VecDeque;

use futures::{Async, Future, Poll, Stream};
use futures::task::{Task as FutureTask, current as current_task};

use h1writer::{Writer, WriterState};
use error::{Error, UnexpectedTaskFrame};
use route::Frame;
use pipeline::MiddlewaresResponse;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

type FrameStream = Stream<Item=Frame, Error=Error>;

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

#[derive(PartialEq, Debug)]
enum TaskIOState {
    ReadingMessage,
    ReadingPayload,
    Done,
}

impl TaskIOState {
    fn is_done(&self) -> bool {
        *self == TaskIOState::Done
    }
}

enum TaskStream {
    None,
    Stream(Box<FrameStream>),
    Context(Box<IoContext<Item=Frame, Error=Error>>),
}

pub(crate) trait IoContext: Stream<Item=Frame, Error=Error> + 'static {
    fn disconnected(&mut self);
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

pub struct Task {
    state: TaskRunningState,
    iostate: TaskIOState,
    frames: VecDeque<Frame>,
    stream: TaskStream,
    drain: Vec<Rc<RefCell<DrainFut>>>,
    prepared: Option<HttpResponse>,
    disconnected: bool,
    middlewares: Option<MiddlewaresResponse>,
}

#[doc(hidden)]
impl Default for Task {

    fn default() -> Task {
        Task { state: TaskRunningState::Running,
               iostate: TaskIOState::ReadingMessage,
               frames: VecDeque::new(),
               drain: Vec::new(),
               stream: TaskStream::None,
               prepared: None,
               disconnected: false,
               middlewares: None }
    }
}

impl Task {

    pub(crate) fn from_response<R: Into<HttpResponse>>(response: R) -> Task {
        let mut frames = VecDeque::new();
        frames.push_back(Frame::Message(response.into()));
        frames.push_back(Frame::Payload(None));

        Task { state: TaskRunningState::Running,
               iostate: TaskIOState::Done,
               frames: frames,
               drain: Vec::new(),
               stream: TaskStream::None,
               prepared: None,
               disconnected: false,
               middlewares: None }
    }

    pub(crate) fn from_error<E: Into<Error>>(err: E) -> Task {
        Task::from_response(err.into())
    }

    pub fn reply<R: Into<HttpResponse>>(&mut self, response: R) {
        self.frames.push_back(Frame::Message(response.into()));
        self.frames.push_back(Frame::Payload(None));
        self.iostate = TaskIOState::Done;
    }

    pub fn error<E: Into<Error>>(&mut self, err: E) {
        self.reply(err.into())
    }

    pub(crate) fn context(&mut self, ctx: Box<IoContext<Item=Frame, Error=Error>>) {
        self.stream = TaskStream::Context(ctx);
    }

    pub fn stream<S>(&mut self, stream: S)
        where S: Stream<Item=Frame, Error=Error> + 'static
    {
        self.stream = TaskStream::Stream(Box::new(stream));
    }

    pub(crate) fn response(&mut self) -> HttpResponse {
        self.prepared.take().unwrap()
    }

    pub(crate) fn set_middlewares(&mut self, middlewares: MiddlewaresResponse) {
        self.middlewares = Some(middlewares)
    }

    pub(crate) fn disconnected(&mut self) {
        self.disconnected = true;
        if let TaskStream::Context(ref mut ctx) = self.stream {
            ctx.disconnected();
        }
    }

    pub(crate) fn poll_io<T>(&mut self, io: &mut T, req: &mut HttpRequest) -> Poll<bool, Error>
        where T: Writer
    {
        trace!("POLL-IO frames:{:?}", self.frames.len());

        // response is completed
        if self.frames.is_empty() && self.iostate.is_done() {
            return Ok(Async::Ready(self.state.is_done()));
        } else if self.drain.is_empty() {
            // poll stream
            if self.state == TaskRunningState::Running {
                match self.poll()? {
                    Async::Ready(_) =>
                        self.state = TaskRunningState::Done,
                    Async::NotReady => (),
                }
            }

            // process middlewares response
            if let Some(mut middlewares) = self.middlewares.take() {
                match middlewares.poll(req)? {
                    Async::NotReady => {
                        self.middlewares = Some(middlewares);
                        return Ok(Async::NotReady);
                    }
                    Async::Ready(None) => {
                        self.middlewares = Some(middlewares);
                    }
                    Async::Ready(Some(mut response)) => {
                        let result = io.start(req, &mut response)?;
                        self.prepared = Some(response);
                        match result {
                            WriterState::Pause => self.state.pause(),
                            WriterState::Done => self.state.resume(),
                        }
                    },
                }
            }

            // if task is paused, write buffer is probably full
            if self.state != TaskRunningState::Paused {
                // process exiting frames
                while let Some(frame) = self.frames.pop_front() {
                    trace!("IO Frame: {:?}", frame);
                    let res = match frame {
                        Frame::Message(mut resp) => {
                            // run middlewares
                            if let Some(mut middlewares) = self.middlewares.take() {
                                match middlewares.response(req, resp) {
                                    Ok(Some(mut resp)) => {
                                        let result = io.start(req, &mut resp)?;
                                        self.prepared = Some(resp);
                                        result
                                    }
                                    Ok(None) => {
                                        // middlewares need to run some futures
                                        self.middlewares = Some(middlewares);
                                        return self.poll_io(io, req)
                                    }
                                    Err(err) => return Err(err),
                                }
                            } else {
                                let result = io.start(req, &mut resp)?;
                                self.prepared = Some(resp);
                                result
                            }
                        }
                        Frame::Payload(Some(chunk)) => {
                            io.write(chunk.as_ref())?
                        },
                        Frame::Payload(None) => {
                            self.iostate = TaskIOState::Done;
                            io.write_eof()?
                        },
                        Frame::Drain(fut) => {
                            self.drain.push(fut);
                            break
                        }
                    };

                    match res {
                        WriterState::Pause => {
                            self.state.pause();
                            break
                        }
                        WriterState::Done => self.state.resume(),
                    }
                }
            }
        }

        // flush io
        match io.poll_complete() {
            Ok(Async::Ready(_)) => self.state.resume(),
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
            if let Some(ref mut resp) = self.prepared {
                resp.set_response_size(io.written());
            }
            Ok(Async::Ready(self.state.is_done()))
        } else {
            Ok(Async::NotReady)
        }
    }

    fn poll_stream<S>(&mut self, stream: &mut S) -> Poll<(), Error>
        where S: Stream<Item=Frame, Error=Error>
    {
        loop {
            match stream.poll() {
                Ok(Async::Ready(Some(frame))) => {
                    match frame {
                        Frame::Message(ref msg) => {
                            if self.iostate != TaskIOState::ReadingMessage {
                                error!("Unexpected frame {:?}", frame);
                                return Err(UnexpectedTaskFrame.into())
                            }
                            let upgrade = msg.upgrade();
                            if upgrade || msg.body().is_streaming() {
                                self.iostate = TaskIOState::ReadingPayload;
                            } else {
                                self.iostate = TaskIOState::Done;
                            }
                        },
                        Frame::Payload(ref chunk) => {
                            if chunk.is_none() {
                                self.iostate = TaskIOState::Done;
                            } else if self.iostate != TaskIOState::ReadingPayload {
                                error!("Unexpected frame {:?}", self.iostate);
                                return Err(UnexpectedTaskFrame.into())
                            }
                        },
                        _ => (),
                    }
                    self.frames.push_back(frame)
                },
                Ok(Async::Ready(None)) =>
                    return Ok(Async::Ready(())),
                Ok(Async::NotReady) =>
                    return Ok(Async::NotReady),
                Err(err) =>
                    return Err(err),
            }
        }
    }

    pub(crate) fn poll(&mut self) -> Poll<(), Error> {
        let mut s = mem::replace(&mut self.stream, TaskStream::None);

        let result = match s {
            TaskStream::None => Ok(Async::Ready(())),
            TaskStream::Stream(ref mut stream) => self.poll_stream(stream),
            TaskStream::Context(ref mut context) => self.poll_stream(context),
        };
        self.stream = s;
        result
    }
}
