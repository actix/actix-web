use std::{mem, io};
use std::rc::Rc;
use std::cell::RefCell;
use std::collections::VecDeque;

use futures::{Async, Future, Poll, Stream};
use futures::task::{Task as FutureTask, current as current_task};

use h1writer::{Writer, WriterState};
use route::Frame;
use application::Middleware;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

type FrameStream = Stream<Item=Frame, Error=io::Error>;

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
    Context(Box<IoContext<Item=Frame, Error=io::Error>>),
}

pub(crate) trait IoContext: Stream<Item=Frame, Error=io::Error> + 'static {
    fn disconnected(&mut self);
}

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
    middlewares: Option<Rc<Vec<Box<Middleware>>>>,
}

impl Task {

    pub fn reply<R: Into<HttpResponse>>(response: R) -> Self {
        let mut frames = VecDeque::new();
        frames.push_back(Frame::Message(response.into()));
        frames.push_back(Frame::Payload(None));

        Task {
            state: TaskRunningState::Running,
            iostate: TaskIOState::Done,
            frames: frames,
            drain: Vec::new(),
            stream: TaskStream::None,
            prepared: None,
            disconnected: false,
            middlewares: None,
        }
    }

    pub(crate) fn with_stream<S>(stream: S) -> Self
        where S: Stream<Item=Frame, Error=io::Error> + 'static
    {
        Task {
            state: TaskRunningState::Running,
            iostate: TaskIOState::ReadingMessage,
            frames: VecDeque::new(),
            stream: TaskStream::Stream(Box::new(stream)),
            drain: Vec::new(),
            prepared: None,
            disconnected: false,
            middlewares: None,
        }
    }

    pub(crate) fn with_context<C: IoContext>(ctx: C) -> Self
    {
        Task {
            state: TaskRunningState::Running,
            iostate: TaskIOState::ReadingMessage,
            frames: VecDeque::new(),
            stream: TaskStream::Context(Box::new(ctx)),
            drain: Vec::new(),
            prepared: None,
            disconnected: false,
            middlewares: None,
        }
    }

    pub(crate) fn set_middlewares(&mut self, middlewares: Rc<Vec<Box<Middleware>>>) {
        self.middlewares = Some(middlewares);
    }

    pub(crate) fn disconnected(&mut self) {
        self.disconnected = true;
        if let TaskStream::Context(ref mut ctx) = self.stream {
            ctx.disconnected();
        }
    }

    pub(crate) fn poll_io<T>(&mut self, io: &mut T, req: &mut HttpRequest) -> Poll<bool, ()>
        where T: Writer
    {
        trace!("POLL-IO frames:{:?}", self.frames.len());
        // response is completed
        if self.frames.is_empty() && self.iostate.is_done() {
            return Ok(Async::Ready(self.state.is_done()));
        } else if self.drain.is_empty() {
            // poll stream
            if self.state == TaskRunningState::Running {
                match self.poll() {
                    Ok(Async::Ready(_)) => {
                        self.state = TaskRunningState::Done;
                    },
                    Ok(Async::NotReady) => (),
                    Err(_) => return Err(())
                }
            }

            // if task is paused, write buffer probably is full
            if self.state != TaskRunningState::Paused {
                // process exiting frames
                while let Some(frame) = self.frames.pop_front() {
                    trace!("IO Frame: {:?}", frame);
                    let res = match frame {
                        Frame::Message(response) => {
                            // run middlewares
                            let mut response =
                                if let Some(middlewares) = self.middlewares.take() {
                                    let mut response = response;
                                    for middleware in middlewares.iter() {
                                        response = middleware.response(req, response);
                                    }
                                    self.middlewares = Some(middlewares);
                                    response
                                } else {
                                    response
                                };

                            let result = io.start(req, &mut response);
                            self.prepared = Some(response);
                            result
                        }
                        Frame::Payload(Some(chunk)) => {
                            io.write(chunk.as_ref())
                        },
                        Frame::Payload(None) => {
                            self.iostate = TaskIOState::Done;
                            io.write_eof()
                        },
                        Frame::Drain(fut) => {
                            self.drain.push(fut);
                            break
                        }
                    };

                    match res {
                        Ok(WriterState::Pause) => {
                            self.state.pause();
                            break
                        }
                        Ok(WriterState::Done) => self.state.resume(),
                        Err(_) => return Err(())
                    }
                }
            }
        }

        // flush io
        match io.poll_complete() {
            Ok(Async::Ready(())) => self.state.resume(),
            Ok(Async::NotReady) => {
                return Ok(Async::NotReady)
            }
            Err(err) => {
                debug!("Error sending data: {}", err);
                return Err(())
            }
        }

        // drain
        if !self.drain.is_empty() {
            for fut in &mut self.drain {
                fut.borrow_mut().set()
            }
            self.drain.clear();
        }

        // response is completed
        if self.iostate.is_done() {
            // run middlewares
            if let Some(ref mut resp) = self.prepared {
                if let Some(middlewares) = self.middlewares.take() {
                    for middleware in middlewares.iter() {
                        middleware.finish(req, resp);
                    }
                }
            }
            Ok(Async::Ready(self.state.is_done()))
        } else {
            Ok(Async::NotReady)
        }
    }

    fn poll_stream<S>(&mut self, stream: &mut S) -> Poll<(), ()>
        where S: Stream<Item=Frame, Error=io::Error>
    {
        loop {
            match stream.poll() {
                Ok(Async::Ready(Some(frame))) => {
                    match frame {
                        Frame::Message(ref msg) => {
                            if self.iostate != TaskIOState::ReadingMessage {
                                error!("Unexpected frame {:?}", frame);
                                return Err(())
                            }
                            let upgrade = msg.upgrade();
                            if upgrade || msg.body().has_body() {
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
                                return Err(())
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
                Err(_) =>
                    return Err(()),
            }
        }
    }
}

impl Future for Task {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
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
