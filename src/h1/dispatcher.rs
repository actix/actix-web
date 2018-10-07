use std::collections::VecDeque;
use std::fmt::{Debug, Display};
use std::time::Instant;

use actix_net::codec::Framed;
use actix_net::service::Service;

use futures::{Async, AsyncSink, Future, Poll, Sink, Stream};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_timer::Delay;

use error::{ParseError, PayloadError};
use payload::{Payload, PayloadSender, PayloadStatus, PayloadWriter};

use body::Body;
use config::ServiceConfig;
use error::DispatchError;
use request::Request;
use response::Response;

use super::codec::{Codec, InMessage, OutMessage};

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
pub struct Dispatcher<T, S: Service>
where
    S::Error: Debug + Display,
{
    service: S,
    flags: Flags,
    framed: Framed<T, Codec>,
    error: Option<DispatchError<S::Error>>,
    config: ServiceConfig,

    state: State<S>,
    payload: Option<PayloadSender>,
    messages: VecDeque<Message>,

    ka_expire: Instant,
    ka_timer: Option<Delay>,
}

enum Message {
    Item(Request),
    Error(Response),
}

enum State<S: Service> {
    None,
    ServiceCall(S::Future),
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

impl<T, S> Dispatcher<T, S>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Request, Response = Response>,
    S::Error: Debug + Display,
{
    /// Create http/1 dispatcher.
    pub fn new(stream: T, config: ServiceConfig, service: S) -> Self {
        Dispatcher::with_timeout(stream, config, None, service)
    }

    /// Create http/1 dispatcher with slow request timeout.
    pub fn with_timeout(
        stream: T, config: ServiceConfig, timeout: Option<Delay>, service: S,
    ) -> Self {
        let keepalive = config.keep_alive_enabled();
        let flags = if keepalive {
            Flags::KEEPALIVE | Flags::KEEPALIVE_ENABLED | Flags::FLUSHED
        } else {
            Flags::FLUSHED
        };
        let framed = Framed::new(stream, Codec::new(keepalive));

        let (ka_expire, ka_timer) = if let Some(delay) = timeout {
            (delay.deadline(), Some(delay))
        } else if let Some(delay) = config.keep_alive_timer() {
            (delay.deadline(), Some(delay))
        } else {
            (config.now(), None)
        };

        Dispatcher {
            payload: None,
            state: State::None,
            error: None,
            messages: VecDeque::new(),
            service,
            flags,
            framed,
            config,
            ka_expire,
            ka_timer,
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
    fn client_disconnected(&mut self) {
        self.flags.insert(Flags::READ_DISCONNECTED);
        if let Some(mut payload) = self.payload.take() {
            payload.set_error(PayloadError::Incomplete);
        }
    }

    /// Flush stream
    fn poll_flush(&mut self) -> Poll<(), DispatchError<S::Error>> {
        if self.flags.contains(Flags::STARTED) && !self.flags.contains(Flags::FLUSHED) {
            match self.framed.poll_complete() {
                Ok(Async::NotReady) => Ok(Async::NotReady),
                Err(err) => {
                    debug!("Error sending data: {}", err);
                    self.client_disconnected();
                    Err(err.into())
                }
                Ok(Async::Ready(_)) => {
                    // if payload is not consumed we can not use connection
                    if self.payload.is_some() && self.state.is_empty() {
                        return Err(DispatchError::PayloadIsNotConsumed);
                    }
                    self.flags.insert(Flags::FLUSHED);
                    Ok(Async::Ready(()))
                }
            }
        } else {
            Ok(Async::Ready(()))
        }
    }

    pub(self) fn poll_handler(&mut self) -> Result<(), DispatchError<S::Error>> {
        self.poll_io()?;
        let mut retry = self.can_read();

        // process
        loop {
            let state = match self.state {
                State::None => loop {
                    break if let Some(msg) = self.messages.pop_front() {
                        match msg {
                            Message::Item(req) => Some(self.handle_request(req)?),
                            Message::Error(res) => Some(State::SendResponse(Some(
                                OutMessage::Response(res),
                            ))),
                        }
                    } else {
                        None
                    };
                },
                State::Payload(ref mut _body) => unimplemented!(),
                State::ServiceCall(ref mut fut) => match fut
                    .poll()
                    .map_err(DispatchError::Service)?
                {
                    Async::Ready(mut res) => {
                        self.framed.get_codec_mut().prepare_te(&mut res);
                        let body = res.replace_body(Body::Empty);
                        if body.is_empty() {
                            Some(State::SendResponse(Some(OutMessage::Response(res))))
                        } else {
                            Some(State::SendResponseWithPayload(Some((
                                OutMessage::Response(res),
                                body,
                            ))))
                        }
                    }
                    Async::NotReady => None,
                },
                State::SendResponse(ref mut item) => {
                    let msg = item.take().expect("SendResponse is empty");
                    match self.framed.start_send(msg) {
                        Ok(AsyncSink::Ready) => {
                            self.flags.set(
                                Flags::KEEPALIVE,
                                self.framed.get_codec().keepalive(),
                            );
                            self.flags.remove(Flags::FLUSHED);
                            Some(State::None)
                        }
                        Ok(AsyncSink::NotReady(msg)) => {
                            *item = Some(msg);
                            return Ok(());
                        }
                        Err(err) => {
                            self.flags.insert(Flags::READ_DISCONNECTED);
                            if let Some(mut payload) = self.payload.take() {
                                payload.set_error(PayloadError::Incomplete);
                            }
                            return Err(DispatchError::Io(err));
                        }
                    }
                }
                State::SendResponseWithPayload(ref mut item) => {
                    let (msg, body) =
                        item.take().expect("SendResponseWithPayload is empty");
                    match self.framed.start_send(msg) {
                        Ok(AsyncSink::Ready) => {
                            self.flags.set(
                                Flags::KEEPALIVE,
                                self.framed.get_codec().keepalive(),
                            );
                            self.flags.remove(Flags::FLUSHED);
                            Some(State::Payload(body))
                        }
                        Ok(AsyncSink::NotReady(msg)) => {
                            *item = Some((msg, body));
                            return Ok(());
                        }
                        Err(err) => {
                            self.flags.insert(Flags::READ_DISCONNECTED);
                            if let Some(mut payload) = self.payload.take() {
                                payload.set_error(PayloadError::Incomplete);
                            }
                            return Err(DispatchError::Io(err));
                        }
                    }
                }
            };

            match state {
                Some(state) => self.state = state,
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

    fn handle_request(
        &mut self, req: Request,
    ) -> Result<State<S>, DispatchError<S::Error>> {
        let mut task = self.service.call(req);
        match task.poll().map_err(DispatchError::Service)? {
            Async::Ready(mut res) => {
                self.framed.get_codec_mut().prepare_te(&mut res);
                let body = res.replace_body(Body::Empty);
                if body.is_empty() {
                    Ok(State::SendResponse(Some(OutMessage::Response(res))))
                } else {
                    Ok(State::SendResponseWithPayload(Some((
                        OutMessage::Response(res),
                        body,
                    ))))
                }
            }
            Async::NotReady => Ok(State::ServiceCall(task)),
        }
    }

    fn one_message(&mut self, msg: InMessage) -> Result<(), DispatchError<S::Error>> {
        self.flags.insert(Flags::STARTED);

        match msg {
            InMessage::Message(msg) => {
                // handle request early
                if self.state.is_empty() {
                    self.state = self.handle_request(msg)?;
                } else {
                    self.messages.push_back(Message::Item(msg));
                }
            }
            InMessage::MessageWithPayload(msg) => {
                // payload
                let (ps, pl) = Payload::new(false);
                *msg.inner.payload.borrow_mut() = Some(pl);
                self.payload = Some(ps);

                self.messages.push_back(Message::Item(msg));
            }
            InMessage::Chunk(chunk) => {
                if let Some(ref mut payload) = self.payload {
                    payload.feed_data(chunk);
                } else {
                    error!("Internal server error: unexpected payload chunk");
                    self.flags.insert(Flags::READ_DISCONNECTED | Flags::STARTED);
                    self.messages.push_back(Message::Error(
                        Response::InternalServerError().finish(),
                    ));
                    self.error = Some(DispatchError::InternalError);
                }
            }
            InMessage::Eof => {
                if let Some(mut payload) = self.payload.take() {
                    payload.feed_eof();
                } else {
                    error!("Internal server error: unexpected eof");
                    self.flags.insert(Flags::READ_DISCONNECTED | Flags::STARTED);
                    self.messages.push_back(Message::Error(
                        Response::InternalServerError().finish(),
                    ));
                    self.error = Some(DispatchError::InternalError);
                }
            }
        }

        Ok(())
    }

    pub(self) fn poll_io(&mut self) -> Result<bool, DispatchError<S::Error>> {
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
                            self.client_disconnected();
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
                        self.messages
                            .push_back(Message::Error(Response::BadRequest().finish()));
                        self.flags.insert(Flags::READ_DISCONNECTED | Flags::STARTED);
                        self.error = Some(DispatchError::MalformedRequest);
                        break;
                    }
                }
            }
        }

        if self.ka_timer.is_some() && updated {
            if let Some(expire) = self.config.keep_alive_expire() {
                self.ka_expire = expire;
            }
        }
        Ok(updated)
    }

    /// keep-alive timer
    fn poll_keepalive(&mut self) -> Result<(), DispatchError<S::Error>> {
        if let Some(ref mut timer) = self.ka_timer {
            match timer.poll() {
                Ok(Async::Ready(_)) => {
                    if timer.deadline() >= self.ka_expire {
                        // check for any outstanding request handling
                        if self.state.is_empty() && self.messages.is_empty() {
                            // if we get timer during shutdown, just drop connection
                            if self.flags.contains(Flags::SHUTDOWN) {
                                return Err(DispatchError::DisconnectTimeout);
                            } else if !self.flags.contains(Flags::STARTED) {
                                // timeout on first request (slow request) return 408
                                trace!("Slow request timeout");
                                self.flags
                                    .insert(Flags::STARTED | Flags::READ_DISCONNECTED);
                                self.state =
                                    State::SendResponse(Some(OutMessage::Response(
                                        Response::RequestTimeout().finish(),
                                    )));
                            } else {
                                trace!("Keep-alive timeout, close connection");
                                self.flags.insert(Flags::SHUTDOWN);

                                // start shutdown timer
                                if let Some(deadline) =
                                    self.config.client_disconnect_timer()
                                {
                                    timer.reset(deadline)
                                } else {
                                    return Ok(());
                                }
                            }
                        } else if let Some(deadline) = self.config.keep_alive_expire() {
                            timer.reset(deadline)
                        }
                    } else {
                        timer.reset(self.ka_expire)
                    }
                }
                Ok(Async::NotReady) => (),
                Err(e) => {
                    error!("Timer error {:?}", e);
                    return Err(DispatchError::Unknown);
                }
            }
        }

        Ok(())
    }
}

impl<T, S> Future for Dispatcher<T, S>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Request, Response = Response>,
    S::Error: Debug + Display,
{
    type Item = ();
    type Error = DispatchError<S::Error>;

    #[inline]
    fn poll(&mut self) -> Poll<(), Self::Error> {
        self.poll_keepalive()?;

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
