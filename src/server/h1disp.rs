// #![allow(unused_imports, unused_variables, dead_code)]
use std::collections::VecDeque;
use std::fmt::{Debug, Display};
use std::net::SocketAddr;
// use std::time::{Duration, Instant};

use actix_net::service::Service;

use futures::{Async, AsyncSink, Future, Poll, Sink, Stream};
use tokio_codec::Framed;
// use tokio_current_thread::spawn;
use tokio_io::AsyncWrite;
// use tokio_timer::Delay;

use error::{ParseError, PayloadError};
use payload::{Payload, PayloadStatus, PayloadWriter};

use body::Body;
use httpresponse::HttpResponse;

use super::error::HttpDispatchError;
use super::h1codec::{H1Codec, OutMessage};
use super::h1decoder::Message;
use super::input::PayloadType;
use super::message::{Request, RequestPool};
use super::IoStream;

const MAX_PIPELINED_MESSAGES: usize = 16;

bitflags! {
    pub struct Flags: u8 {
        const STARTED            = 0b0000_0001;
        const KEEPALIVE_ENABLED  = 0b0000_0010;
        const KEEPALIVE          = 0b0000_0100;
        const SHUTDOWN           = 0b0000_1000;
        const READ_DISCONNECTED  = 0b0001_0000;
        const WRITE_DISCONNECTED = 0b0010_0000;
        const POLLED             = 0b0100_0000;
        const FLUSHED            = 0b1000_0000;
    }
}

/// Dispatcher for HTTP/1.1 protocol
pub struct Http1Dispatcher<T: IoStream, S: Service>
where
    S::Error: Debug + Display,
{
    service: S,
    flags: Flags,
    addr: Option<SocketAddr>,
    framed: Framed<T, H1Codec>,
    error: Option<HttpDispatchError<S::Error>>,

    state: State<S>,
    payload: Option<PayloadType>,
    messages: VecDeque<Request>,
}

enum State<S: Service> {
    None,
    Response(S::Future),
    SendResponse(Option<OutMessage>),
    SendResponseWithPayload(Option<(OutMessage, Body)>),
    Payload(Body),
}

impl<S: Service> State<S> {
    fn is_empty(&self) -> bool {
        if let State::None = self {
            true
        } else {
            false
        }
    }
}

impl<T, S> Http1Dispatcher<T, S>
where
    T: IoStream,
    S: Service<Request = Request, Response = HttpResponse>,
    S::Error: Debug + Display,
{
    pub fn new(stream: T, pool: &'static RequestPool, service: S) -> Self {
        let addr = stream.peer_addr();
        let flags = Flags::FLUSHED;
        let codec = H1Codec::new(pool);
        let framed = Framed::new(stream, codec);

        Http1Dispatcher {
            payload: None,
            state: State::None,
            error: None,
            messages: VecDeque::new(),
            service,
            flags,
            addr,
            framed,
        }
    }

    #[inline]
    fn can_read(&self) -> bool {
        if self.flags.contains(Flags::READ_DISCONNECTED) {
            return false;
        }

        if let Some(ref info) = self.payload {
            info.need_read() == PayloadStatus::Read
        } else {
            true
        }
    }

    // if checked is set to true, delay disconnect until all tasks have finished.
    fn client_disconnected(&mut self, checked: bool) {
        self.flags.insert(Flags::READ_DISCONNECTED);
        if let Some(mut payload) = self.payload.take() {
            payload.set_error(PayloadError::Incomplete);
        }

        // if !checked || self.tasks.is_empty() {
        //     self.flags
        //         .insert(Flags::WRITE_DISCONNECTED | Flags::FLUSHED);

        //     // notify tasks
        //     for mut task in self.tasks.drain(..) {
        //         task.disconnected();
        //         match task.poll_completed() {
        //             Ok(Async::NotReady) => {
        //                 // spawn not completed task, it does not require access to io
        //                 // at this point
        //                 spawn(HttpHandlerTaskFut::new(task.into_task()));
        //             }
        //             Ok(Async::Ready(_)) => (),
        //             Err(err) => {
        //                 error!("Unhandled application error: {}", err);
        //             }
        //         }
        //     }
        // }
    }

    /// Flush stream
    fn poll_flush(&mut self) -> Poll<(), HttpDispatchError<S::Error>> {
        if self.flags.contains(Flags::STARTED) && !self.flags.contains(Flags::FLUSHED) {
            match self.framed.poll_complete() {
                Ok(Async::NotReady) => Ok(Async::NotReady),
                Err(err) => {
                    debug!("Error sending data: {}", err);
                    self.client_disconnected(false);
                    Err(err.into())
                }
                Ok(Async::Ready(_)) => {
                    // if payload is not consumed we can not use connection
                    if self.payload.is_some() && self.state.is_empty() {
                        return Err(HttpDispatchError::PayloadIsNotConsumed);
                    }
                    self.flags.insert(Flags::FLUSHED);
                    Ok(Async::Ready(()))
                }
            }
        } else {
            Ok(Async::Ready(()))
        }
    }

    pub(self) fn poll_handler(&mut self) -> Result<(), HttpDispatchError<S::Error>> {
        self.poll_io()?;
        let mut retry = self.can_read();

        // process
        loop {
            let state = match self.state {
                State::None => loop {
                    break if let Some(msg) = self.messages.pop_front() {
                        let mut task = self.service.call(msg);
                        match task.poll() {
                            Ok(Async::Ready(res)) => {
                                if res.body().is_streaming() {
                                    unimplemented!()
                                } else {
                                    Some(Ok(State::SendResponse(Some(
                                        OutMessage::Response(res),
                                    ))))
                                }
                            }
                            Ok(Async::NotReady) => Some(Ok(State::Response(task))),
                            Err(err) => Some(Err(HttpDispatchError::App(err))),
                        }
                    } else {
                        None
                    };
                },
                State::Payload(ref mut body) => unimplemented!(),
                State::Response(ref mut fut) => {
                    match fut.poll() {
                        Ok(Async::Ready(res)) => {
                            if res.body().is_streaming() {
                                unimplemented!()
                            } else {
                                Some(Ok(State::SendResponse(Some(
                                    OutMessage::Response(res),
                                ))))
                            }
                        }
                        Ok(Async::NotReady) => None,
                        Err(err) => {
                            // it is not possible to recover from error
                            // during pipe handling, so just drop connection
                            Some(Err(HttpDispatchError::App(err)))
                        }
                    }
                }
                State::SendResponse(ref mut item) => {
                    let msg = item.take().expect("SendResponse is empty");
                    match self.framed.start_send(msg) {
                        Ok(AsyncSink::Ready) => {
                            self.flags.remove(Flags::FLUSHED);
                            Some(Ok(State::None))
                        }
                        Ok(AsyncSink::NotReady(msg)) => {
                            *item = Some(msg);
                            return Ok(());
                        }
                        Err(err) => Some(Err(HttpDispatchError::Io(err))),
                    }
                }
                State::SendResponseWithPayload(ref mut item) => {
                    let (msg, body) = item.take().expect("SendResponse is empty");
                    match self.framed.start_send(msg) {
                        Ok(AsyncSink::Ready) => {
                            self.flags.remove(Flags::FLUSHED);
                            Some(Ok(State::Payload(body)))
                        }
                        Ok(AsyncSink::NotReady(msg)) => {
                            *item = Some((msg, body));
                            return Ok(());
                        }
                        Err(err) => Some(Err(HttpDispatchError::Io(err))),
                    }
                }
            };

            match state {
                Some(Ok(state)) => self.state = state,
                Some(Err(err)) => {
                    // error!("Unhandled error1: {}", err);
                    self.client_disconnected(false);
                    return Err(err);
                }
                None => {
                    // if read-backpressure is enabled and we consumed some data.
                    // we may read more dataand retry
                    if !retry && self.can_read() && self.poll_io()? {
                        retry = self.can_read();
                        continue;
                    }
                    break;
                }
            }
        }

        Ok(())
    }

    fn one_message(&mut self, msg: Message) -> Result<(), HttpDispatchError<S::Error>> {
        self.flags.insert(Flags::STARTED);

        match msg {
            Message::Message(mut msg) => {
                // set remote addr
                msg.inner_mut().addr = self.addr;

                // handle request early
                if self.state.is_empty() {
                    let mut task = self.service.call(msg);
                    match task.poll() {
                        Ok(Async::Ready(res)) => {
                            if res.body().is_streaming() {
                                unimplemented!()
                            } else {
                                self.state =
                                    State::SendResponse(Some(OutMessage::Response(res)));
                            }
                        }
                        Ok(Async::NotReady) => self.state = State::Response(task),
                        Err(err) => {
                            error!("Unhandled application error: {}", err);
                            self.client_disconnected(false);
                            return Err(HttpDispatchError::App(err));
                        }
                    }
                } else {
                    self.messages.push_back(msg);
                }
            }
            Message::MessageWithPayload(mut msg) => {
                // set remote addr
                msg.inner_mut().addr = self.addr;

                // payload
                let (ps, pl) = Payload::new(false);
                *msg.inner.payload.borrow_mut() = Some(pl);
                self.payload = Some(PayloadType::new(&msg.inner.headers, ps));

                self.messages.push_back(msg);
            }
            Message::Chunk(chunk) => {
                if let Some(ref mut payload) = self.payload {
                    payload.feed_data(chunk);
                } else {
                    error!("Internal server error: unexpected payload chunk");
                    self.flags.insert(Flags::READ_DISCONNECTED | Flags::STARTED);
                    // self.push_response_entry(StatusCode::INTERNAL_SERVER_ERROR);
                    self.error = Some(HttpDispatchError::InternalError);
                }
            }
            Message::Eof => {
                if let Some(mut payload) = self.payload.take() {
                    payload.feed_eof();
                } else {
                    error!("Internal server error: unexpected eof");
                    self.flags.insert(Flags::READ_DISCONNECTED | Flags::STARTED);
                    // self.push_response_entry(StatusCode::INTERNAL_SERVER_ERROR);
                    self.error = Some(HttpDispatchError::InternalError);
                }
            }
        }

        Ok(())
    }

    pub(self) fn poll_io(&mut self) -> Result<bool, HttpDispatchError<S::Error>> {
        let mut updated = false;

        if self.messages.len() < MAX_PIPELINED_MESSAGES {
            'outer: loop {
                match self.framed.poll() {
                    Ok(Async::Ready(Some(msg))) => {
                        updated = true;
                        self.one_message(msg)?;
                    }
                    Ok(Async::Ready(None)) => {
                        if self.flags.contains(Flags::READ_DISCONNECTED) {
                            self.client_disconnected(true);
                        }
                        break;
                    }
                    Ok(Async::NotReady) => break,
                    Err(e) => {
                        if let Some(mut payload) = self.payload.take() {
                            let e = match e {
                                ParseError::Io(e) => PayloadError::Io(e),
                                _ => PayloadError::EncodingCorrupted,
                            };
                            payload.set_error(e);
                        }

                        // Malformed requests should be responded with 400
                        // self.push_response_entry(StatusCode::BAD_REQUEST);
                        self.flags.insert(Flags::READ_DISCONNECTED | Flags::STARTED);
                        self.error = Some(HttpDispatchError::MalformedRequest);
                        break;
                    }
                }
            }
        }

        Ok(updated)
    }
}

impl<T, S> Future for Http1Dispatcher<T, S>
where
    T: IoStream,
    S: Service<Request = Request, Response = HttpResponse>,
    S::Error: Debug + Display,
{
    type Item = ();
    type Error = HttpDispatchError<S::Error>;

    #[inline]
    fn poll(&mut self) -> Poll<(), Self::Error> {
        // shutdown
        if self.flags.contains(Flags::SHUTDOWN) {
            if self.flags.contains(Flags::WRITE_DISCONNECTED) {
                return Ok(Async::Ready(()));
            }
            try_ready!(self.poll_flush());
            return Ok(AsyncWrite::shutdown(self.framed.get_mut())?);
        }

        // process incoming requests
        if !self.flags.contains(Flags::WRITE_DISCONNECTED) {
            self.poll_handler()?;

            // flush stream
            self.poll_flush()?;

            // deal with keep-alive and stream eof (client-side write shutdown)
            if self.state.is_empty() && self.flags.contains(Flags::FLUSHED) {
                // handle stream eof
                if self
                    .flags
                    .intersects(Flags::READ_DISCONNECTED | Flags::WRITE_DISCONNECTED)
                {
                    return Ok(Async::Ready(()));
                }
                // no keep-alive
                if self.flags.contains(Flags::STARTED)
                    && (!self.flags.contains(Flags::KEEPALIVE_ENABLED)
                        || !self.flags.contains(Flags::KEEPALIVE))
                {
                    self.flags.insert(Flags::SHUTDOWN);
                    return self.poll();
                }
            }
            Ok(Async::NotReady)
        } else if let Some(err) = self.error.take() {
            Err(err)
        } else {
            Ok(Async::Ready(()))
        }
    }
}
