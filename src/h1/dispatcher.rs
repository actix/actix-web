use std::collections::VecDeque;
use std::fmt::Debug;
use std::mem;
use std::time::Instant;

use actix_net::codec::Framed;
use actix_net::service::Service;

use futures::{Async, Future, Poll, Sink, Stream};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_timer::Delay;

use error::{ParseError, PayloadError};
use payload::{Payload, PayloadSender, PayloadStatus, PayloadWriter};

use body::{BodyLength, MessageBody};
use config::ServiceConfig;
use error::DispatchError;
use request::Request;
use response::Response;

use super::codec::Codec;
use super::{H1ServiceResult, Message, MessageType};

const MAX_PIPELINED_MESSAGES: usize = 16;

bitflags! {
    pub struct Flags: u8 {
        const STARTED            = 0b0000_0001;
        const KEEPALIVE_ENABLED  = 0b0000_0010;
        const KEEPALIVE          = 0b0000_0100;
        const POLLED             = 0b0000_1000;
        const FLUSHED            = 0b0001_0000;
        const SHUTDOWN           = 0b0010_0000;
        const DISCONNECTED       = 0b0100_0000;
    }
}

/// Dispatcher for HTTP/1.1 protocol
pub struct Dispatcher<T, S: Service, B: MessageBody>
where
    S::Error: Debug,
{
    inner: Option<InnerDispatcher<T, S, B>>,
}

struct InnerDispatcher<T, S: Service, B: MessageBody>
where
    S::Error: Debug,
{
    service: S,
    flags: Flags,
    framed: Framed<T, Codec>,
    error: Option<DispatchError<S::Error>>,
    config: ServiceConfig,

    state: State<S, B>,
    payload: Option<PayloadSender>,
    messages: VecDeque<DispatcherMessage>,
    unhandled: Option<Request>,

    ka_expire: Instant,
    ka_timer: Option<Delay>,
}

enum DispatcherMessage {
    Item(Request),
    Error(Response),
}

enum State<S: Service, B: MessageBody> {
    None,
    ServiceCall(S::Future),
    SendPayload(B),
}

impl<S: Service, B: MessageBody> State<S, B> {
    fn is_empty(&self) -> bool {
        if let State::None = self {
            true
        } else {
            false
        }
    }
}

impl<T, S, B> Dispatcher<T, S, B>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Request, Response = Response<B>>,
    S::Error: Debug,
    B: MessageBody,
{
    /// Create http/1 dispatcher.
    pub fn new(stream: T, config: ServiceConfig, service: S) -> Self {
        Dispatcher::with_timeout(stream, config, None, service)
    }

    /// Create http/1 dispatcher with slow request timeout.
    pub fn with_timeout(
        stream: T,
        config: ServiceConfig,
        timeout: Option<Delay>,
        service: S,
    ) -> Self {
        let keepalive = config.keep_alive_enabled();
        let flags = if keepalive {
            Flags::KEEPALIVE | Flags::KEEPALIVE_ENABLED | Flags::FLUSHED
        } else {
            Flags::FLUSHED
        };
        let framed = Framed::new(stream, Codec::new(config.clone()));

        // keep-alive timer
        let (ka_expire, ka_timer) = if let Some(delay) = timeout {
            (delay.deadline(), Some(delay))
        } else if let Some(delay) = config.keep_alive_timer() {
            (delay.deadline(), Some(delay))
        } else {
            (config.now(), None)
        };

        Dispatcher {
            inner: Some(InnerDispatcher {
                framed,
                payload: None,
                state: State::None,
                error: None,
                messages: VecDeque::new(),
                unhandled: None,
                service,
                flags,
                config,
                ka_expire,
                ka_timer,
            }),
        }
    }
}

impl<T, S, B> InnerDispatcher<T, S, B>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Request, Response = Response<B>>,
    S::Error: Debug,
    B: MessageBody,
{
    fn can_read(&self) -> bool {
        if self.flags.contains(Flags::DISCONNECTED) {
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
        self.flags.insert(Flags::DISCONNECTED);
        if let Some(mut payload) = self.payload.take() {
            payload.set_error(PayloadError::Incomplete(None));
        }
    }

    /// Flush stream
    fn poll_flush(&mut self) -> Poll<(), DispatchError<S::Error>> {
        if !self.flags.contains(Flags::FLUSHED) {
            match self.framed.poll_complete() {
                Ok(Async::NotReady) => Ok(Async::NotReady),
                Err(err) => {
                    debug!("Error sending data: {}", err);
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

    fn send_response<B1: MessageBody>(
        &mut self,
        message: Response,
        body: B1,
    ) -> Result<State<S, B1>, DispatchError<S::Error>> {
        self.framed
            .force_send(Message::Item(message))
            .map_err(|err| {
                if let Some(mut payload) = self.payload.take() {
                    payload.set_error(PayloadError::Incomplete(None));
                }
                DispatchError::Io(err)
            })?;

        self.flags
            .set(Flags::KEEPALIVE, self.framed.get_codec().keepalive());
        self.flags.remove(Flags::FLUSHED);
        match body.length() {
            BodyLength::None | BodyLength::Zero => Ok(State::None),
            _ => Ok(State::SendPayload(body)),
        }
    }

    fn poll_response(&mut self) -> Result<(), DispatchError<S::Error>> {
        let mut retry = self.can_read();
        loop {
            let state = match mem::replace(&mut self.state, State::None) {
                State::None => match self.messages.pop_front() {
                    Some(DispatcherMessage::Item(req)) => {
                        Some(self.handle_request(req)?)
                    }
                    Some(DispatcherMessage::Error(res)) => {
                        self.send_response(res, ())?;
                        None
                    }
                    None => None,
                },
                State::ServiceCall(mut fut) => {
                    match fut.poll().map_err(DispatchError::Service)? {
                        Async::Ready(mut res) => {
                            let (mut res, body) = res.replace_body(());
                            self.framed
                                .get_codec_mut()
                                .prepare_te(res.head_mut(), &mut body.length());
                            Some(self.send_response(res, body)?)
                        }
                        Async::NotReady => {
                            self.state = State::ServiceCall(fut);
                            None
                        }
                    }
                }
                State::SendPayload(mut stream) => {
                    loop {
                        if !self.framed.is_write_buf_full() {
                            match stream
                                .poll_next()
                                .map_err(|_| DispatchError::Unknown)?
                            {
                                Async::Ready(Some(item)) => {
                                    self.flags.remove(Flags::FLUSHED);
                                    self.framed
                                        .force_send(Message::Chunk(Some(item)))?;
                                    continue;
                                }
                                Async::Ready(None) => {
                                    self.flags.remove(Flags::FLUSHED);
                                    self.framed.force_send(Message::Chunk(None))?;
                                }
                                Async::NotReady => {
                                    self.state = State::SendPayload(stream);
                                    return Ok(());
                                }
                            }
                        } else {
                            self.state = State::SendPayload(stream);
                            return Ok(());
                        }
                        break;
                    }
                    None
                }
            };

            match state {
                Some(state) => self.state = state,
                None => {
                    // if read-backpressure is enabled and we consumed some data.
                    // we may read more data and retry
                    if !retry && self.can_read() && self.poll_request()? {
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
        &mut self,
        req: Request,
    ) -> Result<State<S, B>, DispatchError<S::Error>> {
        let mut task = self.service.call(req);
        match task.poll().map_err(DispatchError::Service)? {
            Async::Ready(res) => {
                let (mut res, body) = res.replace_body(());
                self.framed
                    .get_codec_mut()
                    .prepare_te(res.head_mut(), &mut body.length());
                self.send_response(res, body)
            }
            Async::NotReady => Ok(State::ServiceCall(task)),
        }
    }

    /// Process one incoming requests
    pub(self) fn poll_request(&mut self) -> Result<bool, DispatchError<S::Error>> {
        // limit a mount of non processed requests
        if self.messages.len() >= MAX_PIPELINED_MESSAGES {
            return Ok(false);
        }

        let mut updated = false;
        loop {
            match self.framed.poll() {
                Ok(Async::Ready(Some(msg))) => {
                    updated = true;
                    self.flags.insert(Flags::STARTED);

                    match msg {
                        Message::Item(req) => {
                            match self.framed.get_codec().message_type() {
                                MessageType::Payload => {
                                    let (ps, pl) = Payload::new(false);
                                    *req.inner.payload.borrow_mut() = Some(pl);
                                    self.payload = Some(ps);
                                }
                                MessageType::Stream => {
                                    self.unhandled = Some(req);
                                    return Ok(updated);
                                }
                                _ => (),
                            }

                            // handle request early
                            if self.state.is_empty() {
                                self.state = self.handle_request(req)?;
                            } else {
                                self.messages.push_back(DispatcherMessage::Item(req));
                            }
                        }
                        Message::Chunk(Some(chunk)) => {
                            if let Some(ref mut payload) = self.payload {
                                payload.feed_data(chunk);
                            } else {
                                error!(
                                    "Internal server error: unexpected payload chunk"
                                );
                                self.flags.insert(Flags::DISCONNECTED);
                                self.messages.push_back(DispatcherMessage::Error(
                                    Response::InternalServerError().finish(),
                                ));
                                self.error = Some(DispatchError::InternalError);
                                break;
                            }
                        }
                        Message::Chunk(None) => {
                            if let Some(mut payload) = self.payload.take() {
                                payload.feed_eof();
                            } else {
                                error!("Internal server error: unexpected eof");
                                self.flags.insert(Flags::DISCONNECTED);
                                self.messages.push_back(DispatcherMessage::Error(
                                    Response::InternalServerError().finish(),
                                ));
                                self.error = Some(DispatchError::InternalError);
                                break;
                            }
                        }
                    }
                }
                Ok(Async::Ready(None)) => {
                    self.client_disconnected();
                    break;
                }
                Ok(Async::NotReady) => break,
                Err(ParseError::Io(e)) => {
                    self.client_disconnected();
                    self.error = Some(DispatchError::Io(e));
                    break;
                }
                Err(e) => {
                    if let Some(mut payload) = self.payload.take() {
                        payload.set_error(PayloadError::EncodingCorrupted);
                    }

                    // Malformed requests should be responded with 400
                    self.messages.push_back(DispatcherMessage::Error(
                        Response::BadRequest().finish(),
                    ));
                    self.flags.insert(Flags::DISCONNECTED);
                    self.error = Some(e.into());
                    break;
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
        if self.ka_timer.is_some() {
            return Ok(());
        }
        match self.ka_timer.as_mut().unwrap().poll().map_err(|e| {
            error!("Timer error {:?}", e);
            DispatchError::Unknown
        })? {
            Async::Ready(_) => {
                // if we get timeout during shutdown, drop connection
                if self.flags.contains(Flags::SHUTDOWN) {
                    return Err(DispatchError::DisconnectTimeout);
                } else if self.ka_timer.as_mut().unwrap().deadline() >= self.ka_expire {
                    // check for any outstanding response processing
                    if self.state.is_empty() && self.flags.contains(Flags::FLUSHED) {
                        if self.flags.contains(Flags::STARTED) {
                            trace!("Keep-alive timeout, close connection");
                            self.flags.insert(Flags::SHUTDOWN);

                            // start shutdown timer
                            if let Some(deadline) = self.config.client_disconnect_timer()
                            {
                                self.ka_timer.as_mut().map(|timer| {
                                    timer.reset(deadline);
                                    let _ = timer.poll();
                                });
                            } else {
                                return Ok(());
                            }
                        } else {
                            // timeout on first request (slow request) return 408
                            trace!("Slow request timeout");
                            self.flags.insert(Flags::STARTED | Flags::DISCONNECTED);
                            let _ = self
                                .send_response(Response::RequestTimeout().finish(), ());
                            self.state = State::None;
                        }
                    } else if let Some(deadline) = self.config.keep_alive_expire() {
                        self.ka_timer.as_mut().map(|timer| {
                            timer.reset(deadline);
                            let _ = timer.poll();
                        });
                    }
                } else {
                    let expire = self.ka_expire;
                    self.ka_timer.as_mut().map(|timer| {
                        timer.reset(expire);
                        let _ = timer.poll();
                    });
                }
            }
            Async::NotReady => (),
        }

        Ok(())
    }
}

impl<T, S, B> Future for Dispatcher<T, S, B>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Request, Response = Response<B>>,
    S::Error: Debug,
    B: MessageBody,
{
    type Item = H1ServiceResult<T>;
    type Error = DispatchError<S::Error>;

    #[inline]
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let shutdown = if let Some(ref mut inner) = self.inner {
            if inner.flags.contains(Flags::SHUTDOWN) {
                inner.poll_keepalive()?;
                try_ready!(inner.poll_flush());
                true
            } else {
                inner.poll_keepalive()?;
                inner.poll_request()?;
                inner.poll_response()?;
                inner.poll_flush()?;

                // keep-alive and stream errors
                if inner.state.is_empty() && inner.flags.contains(Flags::FLUSHED) {
                    if let Some(err) = inner.error.take() {
                        return Err(err);
                    } else if inner.flags.contains(Flags::DISCONNECTED) {
                        return Ok(Async::Ready(H1ServiceResult::Disconnected));
                    }
                    // unhandled request (upgrade or connect)
                    else if inner.unhandled.is_some() {
                        false
                    }
                    // disconnect if keep-alive is not enabled
                    else if inner.flags.contains(Flags::STARTED) && !inner
                        .flags
                        .intersects(Flags::KEEPALIVE | Flags::KEEPALIVE_ENABLED)
                    {
                        true
                    } else {
                        return Ok(Async::NotReady);
                    }
                } else {
                    return Ok(Async::NotReady);
                }
            }
        } else {
            unreachable!()
        };

        let mut inner = self.inner.take().unwrap();
        if shutdown {
            Ok(Async::Ready(H1ServiceResult::Shutdown(
                inner.framed.into_inner(),
            )))
        } else {
            let req = inner.unhandled.take().unwrap();
            Ok(Async::Ready(H1ServiceResult::Unhandled(req, inner.framed)))
        }
    }
}
