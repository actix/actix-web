use std::collections::VecDeque;
use std::fmt::Debug;
use std::mem;
use std::time::Instant;

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_service::Service;
use actix_utils::cloneable::CloneableService;
use bitflags::bitflags;
use futures::{Async, Future, Poll, Sink, Stream};
use log::{debug, error, trace};
use tokio_timer::Delay;

use crate::body::{Body, BodyLength, MessageBody, ResponseBody};
use crate::config::ServiceConfig;
use crate::error::DispatchError;
use crate::error::{ParseError, PayloadError};
use crate::request::Request;
use crate::response::Response;

use super::codec::Codec;
use super::payload::{Payload, PayloadSender, PayloadStatus, PayloadWriter};
use super::{Message, MessageType};

const MAX_PIPELINED_MESSAGES: usize = 16;

bitflags! {
    pub struct Flags: u8 {
        const STARTED            = 0b0000_0001;
        const KEEPALIVE_ENABLED  = 0b0000_0010;
        const KEEPALIVE          = 0b0000_0100;
        const POLLED             = 0b0000_1000;
        const SHUTDOWN           = 0b0010_0000;
        const DISCONNECTED       = 0b0100_0000;
        const DROPPING           = 0b1000_0000;
    }
}

/// Dispatcher for HTTP/1.1 protocol
pub struct Dispatcher<T, S: Service<Request = Request> + 'static, B: MessageBody>
where
    S::Error: Debug,
{
    inner: Option<InnerDispatcher<T, S, B>>,
}

struct InnerDispatcher<T, S: Service<Request = Request> + 'static, B: MessageBody>
where
    S::Error: Debug,
{
    service: CloneableService<S>,
    flags: Flags,
    framed: Framed<T, Codec>,
    error: Option<DispatchError>,
    config: ServiceConfig,

    state: State<S, B>,
    payload: Option<PayloadSender>,
    messages: VecDeque<DispatcherMessage>,

    ka_expire: Instant,
    ka_timer: Option<Delay>,
}

enum DispatcherMessage {
    Item(Request),
    Error(Response<()>),
}

enum State<S: Service<Request = Request>, B: MessageBody> {
    None,
    ServiceCall(S::Future),
    SendPayload(ResponseBody<B>),
}

impl<S: Service<Request = Request>, B: MessageBody> State<S, B> {
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
    S: Service<Request = Request> + 'static,
    S::Error: Debug,
    S::Response: Into<Response<B>>,
    B: MessageBody,
{
    /// Create http/1 dispatcher.
    pub fn new(stream: T, config: ServiceConfig, service: CloneableService<S>) -> Self {
        Dispatcher::with_timeout(
            Framed::new(stream, Codec::new(config.clone())),
            config,
            None,
            service,
        )
    }

    /// Create http/1 dispatcher with slow request timeout.
    pub fn with_timeout(
        framed: Framed<T, Codec>,
        config: ServiceConfig,
        timeout: Option<Delay>,
        service: CloneableService<S>,
    ) -> Self {
        let keepalive = config.keep_alive_enabled();
        let flags = if keepalive {
            Flags::KEEPALIVE | Flags::KEEPALIVE_ENABLED
        } else {
            Flags::empty()
        };

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
    S: Service<Request = Request> + 'static,
    S::Error: Debug,
    S::Response: Into<Response<B>>,
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
    fn poll_flush(&mut self) -> Poll<bool, DispatchError> {
        if !self.framed.is_write_buf_empty() {
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
                    Ok(Async::Ready(true))
                }
            }
        } else {
            Ok(Async::Ready(false))
        }
    }

    fn send_response(
        &mut self,
        message: Response<()>,
        body: ResponseBody<B>,
    ) -> Result<State<S, B>, DispatchError> {
        self.framed
            .force_send(Message::Item((message, body.length())))
            .map_err(|err| {
                if let Some(mut payload) = self.payload.take() {
                    payload.set_error(PayloadError::Incomplete(None));
                }
                DispatchError::Io(err)
            })?;

        self.flags
            .set(Flags::KEEPALIVE, self.framed.get_codec().keepalive());
        match body.length() {
            BodyLength::None | BodyLength::Empty => Ok(State::None),
            _ => Ok(State::SendPayload(body)),
        }
    }

    fn poll_response(&mut self) -> Result<(), DispatchError> {
        let mut retry = self.can_read();
        loop {
            let state = match mem::replace(&mut self.state, State::None) {
                State::None => match self.messages.pop_front() {
                    Some(DispatcherMessage::Item(req)) => {
                        Some(self.handle_request(req)?)
                    }
                    Some(DispatcherMessage::Error(res)) => {
                        self.send_response(res, ResponseBody::Other(Body::Empty))?;
                        None
                    }
                    None => None,
                },
                State::ServiceCall(mut fut) => match fut.poll() {
                    Ok(Async::Ready(res)) => {
                        let (res, body) = res.into().replace_body(());
                        Some(self.send_response(res, body)?)
                    }
                    Ok(Async::NotReady) => {
                        self.state = State::ServiceCall(fut);
                        None
                    }
                    Err(_e) => {
                        let res: Response = Response::InternalServerError().finish();
                        let (res, body) = res.replace_body(());
                        Some(self.send_response(res, body.into_body())?)
                    }
                },
                State::SendPayload(mut stream) => {
                    loop {
                        if !self.framed.is_write_buf_full() {
                            match stream
                                .poll_next()
                                .map_err(|_| DispatchError::Unknown)?
                            {
                                Async::Ready(Some(item)) => {
                                    self.framed
                                        .force_send(Message::Chunk(Some(item)))?;
                                    continue;
                                }
                                Async::Ready(None) => {
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

    fn handle_request(&mut self, req: Request) -> Result<State<S, B>, DispatchError> {
        let mut task = self.service.call(req);
        match task.poll() {
            Ok(Async::Ready(res)) => {
                let (res, body) = res.into().replace_body(());
                self.send_response(res, body)
            }
            Ok(Async::NotReady) => Ok(State::ServiceCall(task)),
            Err(_e) => {
                let res: Response = Response::InternalServerError().finish();
                let (res, body) = res.replace_body(());
                self.send_response(res, body.into_body())
            }
        }
    }

    /// Process one incoming requests
    pub(self) fn poll_request(&mut self) -> Result<bool, DispatchError> {
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
                        Message::Item(mut req) => {
                            match self.framed.get_codec().message_type() {
                                MessageType::Payload | MessageType::Stream => {
                                    let (ps, pl) = Payload::create(false);
                                    let (req1, _) =
                                        req.replace_payload(crate::Payload::H1(pl));
                                    req = req1;
                                    self.payload = Some(ps);
                                }
                                //MessageType::Stream => {
                                //    self.unhandled = Some(req);
                                //    return Ok(updated);
                                //}
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
                                    Response::InternalServerError().finish().drop_body(),
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
                                    Response::InternalServerError().finish().drop_body(),
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
                        Response::BadRequest().finish().drop_body(),
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
    fn poll_keepalive(&mut self) -> Result<(), DispatchError> {
        if self.ka_timer.is_none() {
            // shutdown timeout
            if self.flags.contains(Flags::SHUTDOWN) {
                if let Some(interval) = self.config.client_disconnect_timer() {
                    self.ka_timer = Some(Delay::new(interval));
                } else {
                    self.flags.insert(Flags::DISCONNECTED);
                    return Ok(());
                }
            } else {
                return Ok(());
            }
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
                    // check for any outstanding tasks
                    if self.state.is_empty() && self.framed.is_write_buf_empty() {
                        if self.flags.contains(Flags::STARTED) {
                            trace!("Keep-alive timeout, close connection");
                            self.flags.insert(Flags::SHUTDOWN);

                            // start shutdown timer
                            if let Some(deadline) = self.config.client_disconnect_timer()
                            {
                                if let Some(timer) = self.ka_timer.as_mut() {
                                    timer.reset(deadline);
                                    let _ = timer.poll();
                                }
                            } else {
                                // no shutdown timeout, drop socket
                                self.flags.insert(Flags::DISCONNECTED);
                                return Ok(());
                            }
                        } else {
                            // timeout on first request (slow request) return 408
                            if !self.flags.contains(Flags::STARTED) {
                                trace!("Slow request timeout");
                                let _ = self.send_response(
                                    Response::RequestTimeout().finish().drop_body(),
                                    ResponseBody::Other(Body::Empty),
                                );
                            } else {
                                trace!("Keep-alive connection timeout");
                            }
                            self.flags.insert(Flags::STARTED | Flags::SHUTDOWN);
                            self.state = State::None;
                        }
                    } else if let Some(deadline) = self.config.keep_alive_expire() {
                        if let Some(timer) = self.ka_timer.as_mut() {
                            timer.reset(deadline);
                            let _ = timer.poll();
                        }
                    }
                } else if let Some(timer) = self.ka_timer.as_mut() {
                    timer.reset(self.ka_expire);
                    let _ = timer.poll();
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
    S: Service<Request = Request>,
    S::Error: Debug,
    S::Response: Into<Response<B>>,
    B: MessageBody,
{
    type Item = ();
    type Error = DispatchError;

    #[inline]
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let inner = self.inner.as_mut().unwrap();

        if inner.flags.contains(Flags::SHUTDOWN) {
            inner.poll_keepalive()?;
            if inner.flags.contains(Flags::DISCONNECTED) {
                Ok(Async::Ready(()))
            } else {
                // try_ready!(inner.poll_flush());
                match inner.framed.get_mut().shutdown()? {
                    Async::Ready(_) => Ok(Async::Ready(())),
                    Async::NotReady => Ok(Async::NotReady),
                }
            }
        } else {
            inner.poll_keepalive()?;
            inner.poll_request()?;
            loop {
                inner.poll_response()?;
                if let Async::Ready(false) = inner.poll_flush()? {
                    break;
                }
            }

            if inner.flags.contains(Flags::DISCONNECTED) {
                return Ok(Async::Ready(()));
            }

            // keep-alive and stream errors
            if inner.state.is_empty() && inner.framed.is_write_buf_empty() {
                if let Some(err) = inner.error.take() {
                    return Err(err);
                }
                // disconnect if keep-alive is not enabled
                else if inner.flags.contains(Flags::STARTED)
                    && !inner.flags.intersects(Flags::KEEPALIVE)
                {
                    inner.flags.insert(Flags::SHUTDOWN);
                    self.poll()
                }
                // disconnect if shutdown
                else if inner.flags.contains(Flags::SHUTDOWN) {
                    self.poll()
                } else {
                    return Ok(Async::NotReady);
                }
            } else {
                return Ok(Async::NotReady);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cmp, io};

    use actix_codec::{AsyncRead, AsyncWrite};
    use actix_service::IntoService;
    use bytes::{Buf, Bytes, BytesMut};
    use futures::future::{lazy, ok};

    use super::*;
    use crate::error::Error;

    struct Buffer {
        buf: Bytes,
        err: Option<io::Error>,
    }

    impl Buffer {
        fn new(data: &'static str) -> Buffer {
            Buffer {
                buf: Bytes::from(data),
                err: None,
            }
        }
    }

    impl AsyncRead for Buffer {}
    impl io::Read for Buffer {
        fn read(&mut self, dst: &mut [u8]) -> Result<usize, io::Error> {
            if self.buf.is_empty() {
                if self.err.is_some() {
                    Err(self.err.take().unwrap())
                } else {
                    Err(io::Error::new(io::ErrorKind::WouldBlock, ""))
                }
            } else {
                let size = cmp::min(self.buf.len(), dst.len());
                let b = self.buf.split_to(size);
                dst[..size].copy_from_slice(&b);
                Ok(size)
            }
        }
    }

    impl io::Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl AsyncWrite for Buffer {
        fn shutdown(&mut self) -> Poll<(), io::Error> {
            Ok(Async::Ready(()))
        }
        fn write_buf<B: Buf>(&mut self, _: &mut B) -> Poll<usize, io::Error> {
            Ok(Async::NotReady)
        }
    }

    #[test]
    fn test_req_parse_err() {
        let mut sys = actix_rt::System::new("test");
        let _ = sys.block_on(lazy(|| {
            let buf = Buffer::new("GET /test HTTP/1\r\n\r\n");

            let mut h1 = Dispatcher::new(
                buf,
                ServiceConfig::default(),
                CloneableService::new(
                    (|_| ok::<_, Error>(Response::Ok().finish())).into_service(),
                ),
            );
            assert!(h1.poll().is_ok());
            assert!(h1.poll().is_ok());
            assert!(h1
                .inner
                .as_ref()
                .unwrap()
                .flags
                .contains(Flags::DISCONNECTED));
            // assert_eq!(h1.tasks.len(), 1);
            ok::<_, ()>(())
        }));
    }
}
