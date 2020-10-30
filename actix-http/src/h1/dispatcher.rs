use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::{fmt, io, net};

use actix_codec::{AsyncRead, AsyncWrite, Decoder, Encoder, Framed, FramedParts};
use actix_rt::time::{delay_until, Delay, Instant};
use actix_service::Service;
use bitflags::bitflags;
use bytes::{Buf, BytesMut};
use log::{error, trace};
use pin_project::pin_project;

use crate::cloneable::CloneableService;
use crate::config::ServiceConfig;
use crate::error::{DispatchError, Error};
use crate::error::{ParseError, PayloadError};
use crate::helpers::DataFactory;
use crate::httpmessage::HttpMessage;
use crate::request::Request;
use crate::response::Response;
use crate::{
    body::{Body, BodySize, MessageBody, ResponseBody},
    Extensions,
};

use super::codec::Codec;
use super::payload::{Payload, PayloadSender, PayloadStatus};
use super::{Message, MessageType};

const LW_BUFFER_SIZE: usize = 4096;
const HW_BUFFER_SIZE: usize = 32_768;
const MAX_PIPELINED_MESSAGES: usize = 16;

bitflags! {
    pub struct Flags: u8 {
        const STARTED            = 0b0000_0001;
        const KEEPALIVE          = 0b0000_0010;
        const POLLED             = 0b0000_0100;
        const SHUTDOWN           = 0b0000_1000;
        const READ_DISCONNECT    = 0b0001_0000;
        const WRITE_DISCONNECT   = 0b0010_0000;
        const UPGRADE            = 0b0100_0000;
    }
}

#[pin_project::pin_project]
/// Dispatcher for HTTP/1.1 protocol
pub struct Dispatcher<T, S, B, X, U>
where
    S: Service<Request = Request>,
    S::Error: Into<Error>,
    B: MessageBody,
    X: Service<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    U: Service<Request = (Request, Framed<T, Codec>), Response = ()>,
    U::Error: fmt::Display,
{
    #[pin]
    inner: DispatcherState<T, S, B, X, U>,
}

#[pin_project(project = DispatcherStateProj)]
enum DispatcherState<T, S, B, X, U>
where
    S: Service<Request = Request>,
    S::Error: Into<Error>,
    B: MessageBody,
    X: Service<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    U: Service<Request = (Request, Framed<T, Codec>), Response = ()>,
    U::Error: fmt::Display,
{
    Normal(#[pin] InnerDispatcher<T, S, B, X, U>),
    Upgrade(Pin<Box<U::Future>>),
}

#[pin_project(project = InnerDispatcherProj)]
struct InnerDispatcher<T, S, B, X, U>
where
    S: Service<Request = Request>,
    S::Error: Into<Error>,
    B: MessageBody,
    X: Service<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    U: Service<Request = (Request, Framed<T, Codec>), Response = ()>,
    U::Error: fmt::Display,
{
    service: CloneableService<S>,
    expect: CloneableService<X>,
    upgrade: Option<CloneableService<U>>,
    on_connect: Option<Box<dyn DataFactory>>,
    on_connect_data: Extensions,
    flags: Flags,
    peer_addr: Option<net::SocketAddr>,
    error: Option<DispatchError>,

    #[pin]
    state: State<S, B, X>,
    payload: Option<PayloadSender>,
    messages: VecDeque<DispatcherMessage>,

    ka_expire: Instant,
    ka_timer: Option<Delay>,

    io: Option<T>,
    read_buf: BytesMut,
    write_buf: BytesMut,
    codec: Codec,
}

enum DispatcherMessage {
    Item(Request),
    Upgrade(Request),
    Error(Response<()>),
}

#[pin_project(project = StateProj)]
enum State<S, B, X>
where
    S: Service<Request = Request>,
    X: Service<Request = Request, Response = Request>,
    B: MessageBody,
{
    None,
    ExpectCall(Pin<Box<X::Future>>),
    ServiceCall(Pin<Box<S::Future>>),
    SendPayload(#[pin] ResponseBody<B>),
}

impl<S, B, X> State<S, B, X>
where
    S: Service<Request = Request>,
    X: Service<Request = Request, Response = Request>,
    B: MessageBody,
{
    fn is_empty(&self) -> bool {
        matches!(self, State::None)
    }

    fn is_call(&self) -> bool {
        matches!(self, State::ServiceCall(_))
    }
}
enum PollResponse {
    Upgrade(Request),
    DoNothing,
    DrainWriteBuf,
}

impl PartialEq for PollResponse {
    fn eq(&self, other: &PollResponse) -> bool {
        match self {
            PollResponse::DrainWriteBuf => matches!(other, PollResponse::DrainWriteBuf),
            PollResponse::DoNothing => matches!(other, PollResponse::DoNothing),
            _ => false,
        }
    }
}

impl<T, S, B, X, U> Dispatcher<T, S, B, X, U>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: Service<Request = Request>,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    B: MessageBody,
    X: Service<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    U: Service<Request = (Request, Framed<T, Codec>), Response = ()>,
    U::Error: fmt::Display,
{
    /// Create HTTP/1 dispatcher.
    pub(crate) fn new(
        stream: T,
        config: ServiceConfig,
        service: CloneableService<S>,
        expect: CloneableService<X>,
        upgrade: Option<CloneableService<U>>,
        on_connect: Option<Box<dyn DataFactory>>,
        on_connect_data: Extensions,
        peer_addr: Option<net::SocketAddr>,
    ) -> Self {
        Dispatcher::with_timeout(
            stream,
            Codec::new(config.clone()),
            config,
            BytesMut::with_capacity(HW_BUFFER_SIZE),
            None,
            service,
            expect,
            upgrade,
            on_connect,
            on_connect_data,
            peer_addr,
        )
    }

    /// Create http/1 dispatcher with slow request timeout.
    pub(crate) fn with_timeout(
        io: T,
        codec: Codec,
        config: ServiceConfig,
        read_buf: BytesMut,
        timeout: Option<Delay>,
        service: CloneableService<S>,
        expect: CloneableService<X>,
        upgrade: Option<CloneableService<U>>,
        on_connect: Option<Box<dyn DataFactory>>,
        on_connect_data: Extensions,
        peer_addr: Option<net::SocketAddr>,
    ) -> Self {
        let keepalive = config.keep_alive_enabled();
        let flags = if keepalive {
            Flags::KEEPALIVE
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
            inner: DispatcherState::Normal(InnerDispatcher {
                write_buf: BytesMut::with_capacity(HW_BUFFER_SIZE),
                payload: None,
                state: State::None,
                error: None,
                messages: VecDeque::new(),
                io: Some(io),
                codec,
                read_buf,
                service,
                expect,
                upgrade,
                on_connect,
                on_connect_data,
                flags,
                peer_addr,
                ka_expire,
                ka_timer,
            }),
        }
    }
}

impl<T, S, B, X, U> InnerDispatcher<T, S, B, X, U>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: Service<Request = Request>,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    B: MessageBody,
    X: Service<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    U: Service<Request = (Request, Framed<T, Codec>), Response = ()>,
    U::Error: fmt::Display,
{
    fn can_read(&self, cx: &mut Context<'_>) -> bool {
        if self
            .flags
            .intersects(Flags::READ_DISCONNECT | Flags::UPGRADE)
        {
            false
        } else if let Some(ref info) = self.payload {
            info.need_read(cx) == PayloadStatus::Read
        } else {
            true
        }
    }

    // if checked is set to true, delay disconnect until all tasks have finished.
    fn client_disconnected(self: Pin<&mut Self>) {
        let this = self.project();
        this.flags
            .insert(Flags::READ_DISCONNECT | Flags::WRITE_DISCONNECT);
        if let Some(mut payload) = this.payload.take() {
            payload.set_error(PayloadError::Incomplete(None));
        }
    }

    /// Flush stream
    ///
    /// true - got WouldBlock
    /// false - didn't get WouldBlock
    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Result<bool, DispatchError> {
        if self.write_buf.is_empty() {
            return Ok(false);
        }

        let len = self.write_buf.len();
        let mut written = 0;
        let InnerDispatcherProj { io, write_buf, .. } = self.project();
        let mut io = Pin::new(io.as_mut().unwrap());
        while written < len {
            match io.as_mut().poll_write(cx, &write_buf[written..]) {
                Poll::Ready(Ok(0)) => {
                    return Err(DispatchError::Io(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "",
                    )));
                }
                Poll::Ready(Ok(n)) => {
                    written += n;
                }
                Poll::Pending => {
                    if written > 0 {
                        write_buf.advance(written);
                    }
                    return Ok(true);
                }
                Poll::Ready(Err(err)) => return Err(DispatchError::Io(err)),
            }
        }

        if written == write_buf.len() {
            // SAFETY: setting length to 0 is safe
            // skips one length check vs truncate
            unsafe { write_buf.set_len(0) }
        } else {
            write_buf.advance(written);
        }

        Ok(false)
    }

    fn send_response(
        self: Pin<&mut Self>,
        message: Response<()>,
        body: ResponseBody<B>,
    ) -> Result<State<S, B, X>, DispatchError> {
        let mut this = self.project();
        this.codec
            .encode(Message::Item((message, body.size())), &mut this.write_buf)
            .map_err(|err| {
                if let Some(mut payload) = this.payload.take() {
                    payload.set_error(PayloadError::Incomplete(None));
                }
                DispatchError::Io(err)
            })?;

        this.flags.set(Flags::KEEPALIVE, this.codec.keepalive());
        match body.size() {
            BodySize::None | BodySize::Empty => Ok(State::None),
            _ => Ok(State::SendPayload(body)),
        }
    }

    fn send_continue(self: Pin<&mut Self>) {
        self.project()
            .write_buf
            .extend_from_slice(b"HTTP/1.1 100 Continue\r\n\r\n");
    }

    fn poll_response(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Result<PollResponse, DispatchError> {
        loop {
            let mut this = self.as_mut().project();
            let state = match this.state.project() {
                StateProj::None => match this.messages.pop_front() {
                    Some(DispatcherMessage::Item(req)) => {
                        Some(self.as_mut().handle_request(req, cx)?)
                    }
                    Some(DispatcherMessage::Error(res)) => Some(
                        self.as_mut()
                            .send_response(res, ResponseBody::Other(Body::Empty))?,
                    ),
                    Some(DispatcherMessage::Upgrade(req)) => {
                        return Ok(PollResponse::Upgrade(req));
                    }
                    None => None,
                },
                StateProj::ExpectCall(fut) => match fut.as_mut().poll(cx) {
                    Poll::Ready(Ok(req)) => {
                        self.as_mut().send_continue();
                        this = self.as_mut().project();
                        this.state
                            .set(State::ServiceCall(Box::pin(this.service.call(req))));
                        continue;
                    }
                    Poll::Ready(Err(e)) => {
                        let res: Response = e.into().into();
                        let (res, body) = res.replace_body(());
                        Some(self.as_mut().send_response(res, body.into_body())?)
                    }
                    Poll::Pending => None,
                },
                StateProj::ServiceCall(fut) => match fut.as_mut().poll(cx) {
                    Poll::Ready(Ok(res)) => {
                        let (res, body) = res.into().replace_body(());
                        let state = self.as_mut().send_response(res, body)?;
                        this = self.as_mut().project();
                        this.state.set(state);
                        continue;
                    }
                    Poll::Ready(Err(e)) => {
                        let res: Response = e.into().into();
                        let (res, body) = res.replace_body(());
                        Some(self.as_mut().send_response(res, body.into_body())?)
                    }
                    Poll::Pending => None,
                },
                StateProj::SendPayload(mut stream) => {
                    loop {
                        if this.write_buf.len() < HW_BUFFER_SIZE {
                            match stream.as_mut().poll_next(cx) {
                                Poll::Ready(Some(Ok(item))) => {
                                    this.codec.encode(
                                        Message::Chunk(Some(item)),
                                        &mut this.write_buf,
                                    )?;
                                    continue;
                                }
                                Poll::Ready(None) => {
                                    this.codec.encode(
                                        Message::Chunk(None),
                                        &mut this.write_buf,
                                    )?;
                                    this = self.as_mut().project();
                                    this.state.set(State::None);
                                }
                                Poll::Ready(Some(Err(_))) => {
                                    return Err(DispatchError::Unknown)
                                }
                                Poll::Pending => return Ok(PollResponse::DoNothing),
                            }
                        } else {
                            return Ok(PollResponse::DrainWriteBuf);
                        }
                        break;
                    }
                    continue;
                }
            };

            this = self.as_mut().project();

            // set new state
            if let Some(state) = state {
                this.state.set(state);
                if !self.state.is_empty() {
                    continue;
                }
            } else {
                // if read-backpressure is enabled and we consumed some data.
                // we may read more data and retry
                if self.state.is_call() {
                    if self.as_mut().poll_request(cx)? {
                        continue;
                    }
                } else if !self.messages.is_empty() {
                    continue;
                }
            }
            break;
        }

        Ok(PollResponse::DoNothing)
    }

    fn handle_request(
        mut self: Pin<&mut Self>,
        req: Request,
        cx: &mut Context<'_>,
    ) -> Result<State<S, B, X>, DispatchError> {
        // Handle `EXPECT: 100-Continue` header
        let req = if req.head().expect() {
            let mut task = Box::pin(self.as_mut().project().expect.call(req));
            match task.as_mut().poll(cx) {
                Poll::Ready(Ok(req)) => {
                    self.as_mut().send_continue();
                    req
                }
                Poll::Pending => return Ok(State::ExpectCall(task)),
                Poll::Ready(Err(e)) => {
                    let e = e.into();
                    let res: Response = e.into();
                    let (res, body) = res.replace_body(());
                    return self.send_response(res, body.into_body());
                }
            }
        } else {
            req
        };

        // Call service
        let mut task = Box::pin(self.as_mut().project().service.call(req));
        match task.as_mut().poll(cx) {
            Poll::Ready(Ok(res)) => {
                let (res, body) = res.into().replace_body(());
                self.send_response(res, body)
            }
            Poll::Pending => Ok(State::ServiceCall(task)),
            Poll::Ready(Err(e)) => {
                let res: Response = e.into().into();
                let (res, body) = res.replace_body(());
                self.send_response(res, body.into_body())
            }
        }
    }

    /// Process one incoming requests
    pub(self) fn poll_request(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Result<bool, DispatchError> {
        // limit a mount of non processed requests
        if self.messages.len() >= MAX_PIPELINED_MESSAGES || !self.can_read(cx) {
            return Ok(false);
        }

        let mut updated = false;
        let mut this = self.as_mut().project();
        loop {
            match this.codec.decode(&mut this.read_buf) {
                Ok(Some(msg)) => {
                    updated = true;
                    this.flags.insert(Flags::STARTED);

                    match msg {
                        Message::Item(mut req) => {
                            let pl = this.codec.message_type();
                            req.head_mut().peer_addr = *this.peer_addr;

                            // DEPRECATED
                            // set on_connect data
                            if let Some(ref on_connect) = this.on_connect {
                                on_connect.set(&mut req.extensions_mut());
                            }

                            // merge on_connect_ext data into request extensions
                            req.extensions_mut().drain_from(this.on_connect_data);

                            if pl == MessageType::Stream && this.upgrade.is_some() {
                                this.messages.push_back(DispatcherMessage::Upgrade(req));
                                break;
                            }
                            if pl == MessageType::Payload || pl == MessageType::Stream {
                                let (ps, pl) = Payload::create(false);
                                let (req1, _) =
                                    req.replace_payload(crate::Payload::H1(pl));
                                req = req1;
                                *this.payload = Some(ps);
                            }

                            // handle request early
                            if this.state.is_empty() {
                                let state = self.as_mut().handle_request(req, cx)?;
                                this = self.as_mut().project();
                                this.state.set(state);
                            } else {
                                this.messages.push_back(DispatcherMessage::Item(req));
                            }
                        }
                        Message::Chunk(Some(chunk)) => {
                            if let Some(ref mut payload) = this.payload {
                                payload.feed_data(chunk);
                            } else {
                                error!(
                                    "Internal server error: unexpected payload chunk"
                                );
                                this.flags.insert(Flags::READ_DISCONNECT);
                                this.messages.push_back(DispatcherMessage::Error(
                                    Response::InternalServerError().finish().drop_body(),
                                ));
                                *this.error = Some(DispatchError::InternalError);
                                break;
                            }
                        }
                        Message::Chunk(None) => {
                            if let Some(mut payload) = this.payload.take() {
                                payload.feed_eof();
                            } else {
                                error!("Internal server error: unexpected eof");
                                this.flags.insert(Flags::READ_DISCONNECT);
                                this.messages.push_back(DispatcherMessage::Error(
                                    Response::InternalServerError().finish().drop_body(),
                                ));
                                *this.error = Some(DispatchError::InternalError);
                                break;
                            }
                        }
                    }
                }
                Ok(None) => break,
                Err(ParseError::Io(e)) => {
                    self.as_mut().client_disconnected();
                    this = self.as_mut().project();
                    *this.error = Some(DispatchError::Io(e));
                    break;
                }
                Err(e) => {
                    if let Some(mut payload) = this.payload.take() {
                        payload.set_error(PayloadError::EncodingCorrupted);
                    }

                    // Malformed requests should be responded with 400
                    this.messages.push_back(DispatcherMessage::Error(
                        Response::BadRequest().finish().drop_body(),
                    ));
                    this.flags.insert(Flags::READ_DISCONNECT);
                    *this.error = Some(e.into());
                    break;
                }
            }
        }

        if updated && this.ka_timer.is_some() {
            if let Some(expire) = this.codec.config().keep_alive_expire() {
                *this.ka_expire = expire;
            }
        }
        Ok(updated)
    }

    /// keep-alive timer
    fn poll_keepalive(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Result<(), DispatchError> {
        let mut this = self.as_mut().project();
        if this.ka_timer.is_none() {
            // shutdown timeout
            if this.flags.contains(Flags::SHUTDOWN) {
                if let Some(interval) = this.codec.config().client_disconnect_timer() {
                    *this.ka_timer = Some(delay_until(interval));
                } else {
                    this.flags.insert(Flags::READ_DISCONNECT);
                    if let Some(mut payload) = this.payload.take() {
                        payload.set_error(PayloadError::Incomplete(None));
                    }
                    return Ok(());
                }
            } else {
                return Ok(());
            }
        }

        match Pin::new(&mut this.ka_timer.as_mut().unwrap()).poll(cx) {
            Poll::Ready(()) => {
                // if we get timeout during shutdown, drop connection
                if this.flags.contains(Flags::SHUTDOWN) {
                    return Err(DispatchError::DisconnectTimeout);
                } else if this.ka_timer.as_mut().unwrap().deadline() >= *this.ka_expire {
                    // check for any outstanding tasks
                    if this.state.is_empty() && this.write_buf.is_empty() {
                        if this.flags.contains(Flags::STARTED) {
                            trace!("Keep-alive timeout, close connection");
                            this.flags.insert(Flags::SHUTDOWN);

                            // start shutdown timer
                            if let Some(deadline) =
                                this.codec.config().client_disconnect_timer()
                            {
                                if let Some(mut timer) = this.ka_timer.as_mut() {
                                    timer.reset(deadline);
                                    let _ = Pin::new(&mut timer).poll(cx);
                                }
                            } else {
                                // no shutdown timeout, drop socket
                                this.flags.insert(Flags::WRITE_DISCONNECT);
                                return Ok(());
                            }
                        } else {
                            // timeout on first request (slow request) return 408
                            if !this.flags.contains(Flags::STARTED) {
                                trace!("Slow request timeout");
                                let _ = self.as_mut().send_response(
                                    Response::RequestTimeout().finish().drop_body(),
                                    ResponseBody::Other(Body::Empty),
                                );
                                this = self.as_mut().project();
                            } else {
                                trace!("Keep-alive connection timeout");
                            }
                            this.flags.insert(Flags::STARTED | Flags::SHUTDOWN);
                            this.state.set(State::None);
                        }
                    } else if let Some(deadline) =
                        this.codec.config().keep_alive_expire()
                    {
                        if let Some(mut timer) = this.ka_timer.as_mut() {
                            timer.reset(deadline);
                            let _ = Pin::new(&mut timer).poll(cx);
                        }
                    }
                } else if let Some(mut timer) = this.ka_timer.as_mut() {
                    timer.reset(*this.ka_expire);
                    let _ = Pin::new(&mut timer).poll(cx);
                }
            }
            Poll::Pending => (),
        }

        Ok(())
    }
}

impl<T, S, B, X, U> Future for Dispatcher<T, S, B, X, U>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: Service<Request = Request>,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    B: MessageBody,
    X: Service<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    U: Service<Request = (Request, Framed<T, Codec>), Response = ()>,
    U::Error: fmt::Display,
{
    type Output = Result<(), DispatchError>;

    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().project();
        match this.inner.project() {
            DispatcherStateProj::Normal(mut inner) => {
                inner.as_mut().poll_keepalive(cx)?;

                if inner.flags.contains(Flags::SHUTDOWN) {
                    if inner.flags.contains(Flags::WRITE_DISCONNECT) {
                        Poll::Ready(Ok(()))
                    } else {
                        // flush buffer
                        inner.as_mut().poll_flush(cx)?;
                        if !inner.write_buf.is_empty() || inner.io.is_none() {
                            Poll::Pending
                        } else {
                            match Pin::new(inner.project().io)
                                .as_pin_mut()
                                .unwrap()
                                .poll_shutdown(cx)
                            {
                                Poll::Ready(res) => {
                                    Poll::Ready(res.map_err(DispatchError::from))
                                }
                                Poll::Pending => Poll::Pending,
                            }
                        }
                    }
                } else {
                    // read socket into a buf
                    let should_disconnect =
                        if !inner.flags.contains(Flags::READ_DISCONNECT) {
                            let mut inner_p = inner.as_mut().project();
                            read_available(
                                cx,
                                inner_p.io.as_mut().unwrap(),
                                &mut inner_p.read_buf,
                            )?
                        } else {
                            None
                        };

                    inner.as_mut().poll_request(cx)?;
                    if let Some(true) = should_disconnect {
                        let inner_p = inner.as_mut().project();
                        inner_p.flags.insert(Flags::READ_DISCONNECT);
                        if let Some(mut payload) = inner_p.payload.take() {
                            payload.feed_eof();
                        }
                    };

                    loop {
                        let inner_p = inner.as_mut().project();
                        let remaining =
                            inner_p.write_buf.capacity() - inner_p.write_buf.len();
                        if remaining < LW_BUFFER_SIZE {
                            inner_p.write_buf.reserve(HW_BUFFER_SIZE - remaining);
                        }
                        let result = inner.as_mut().poll_response(cx)?;
                        let drain = result == PollResponse::DrainWriteBuf;

                        // switch to upgrade handler
                        if let PollResponse::Upgrade(req) = result {
                            let inner_p = inner.as_mut().project();
                            let mut parts = FramedParts::with_read_buf(
                                inner_p.io.take().unwrap(),
                                std::mem::take(inner_p.codec),
                                std::mem::take(inner_p.read_buf),
                            );
                            parts.write_buf = std::mem::take(inner_p.write_buf);
                            let framed = Framed::from_parts(parts);
                            let upgrade =
                                inner_p.upgrade.take().unwrap().call((req, framed));
                            self.as_mut()
                                .project()
                                .inner
                                .set(DispatcherState::Upgrade(Box::pin(upgrade)));
                            return self.poll(cx);
                        }

                        // we didn't get WouldBlock from write operation,
                        // so data get written to kernel completely (OSX)
                        // and we have to write again otherwise response can get stuck
                        if inner.as_mut().poll_flush(cx)? || !drain {
                            break;
                        }
                    }

                    // client is gone
                    if inner.flags.contains(Flags::WRITE_DISCONNECT) {
                        return Poll::Ready(Ok(()));
                    }

                    let is_empty = inner.state.is_empty();

                    let inner_p = inner.as_mut().project();
                    // read half is closed and we do not processing any responses
                    if inner_p.flags.contains(Flags::READ_DISCONNECT) && is_empty {
                        inner_p.flags.insert(Flags::SHUTDOWN);
                    }

                    // keep-alive and stream errors
                    if is_empty && inner_p.write_buf.is_empty() {
                        if let Some(err) = inner_p.error.take() {
                            Poll::Ready(Err(err))
                        }
                        // disconnect if keep-alive is not enabled
                        else if inner_p.flags.contains(Flags::STARTED)
                            && !inner_p.flags.intersects(Flags::KEEPALIVE)
                        {
                            inner_p.flags.insert(Flags::SHUTDOWN);
                            self.poll(cx)
                        }
                        // disconnect if shutdown
                        else if inner_p.flags.contains(Flags::SHUTDOWN) {
                            self.poll(cx)
                        } else {
                            Poll::Pending
                        }
                    } else {
                        Poll::Pending
                    }
                }
            }
            DispatcherStateProj::Upgrade(fut) => fut.as_mut().poll(cx).map_err(|e| {
                error!("Upgrade handler error: {}", e);
                DispatchError::Upgrade
            }),
        }
    }
}

fn read_available<T>(
    cx: &mut Context<'_>,
    io: &mut T,
    buf: &mut BytesMut,
) -> Result<Option<bool>, io::Error>
where
    T: AsyncRead + Unpin,
{
    let mut read_some = false;

    loop {
        // If buf is full return but do not disconnect since
        // there is more reading to be done
        if buf.len() >= HW_BUFFER_SIZE {
            return Ok(Some(false));
        }

        let remaining = buf.capacity() - buf.len();
        if remaining < LW_BUFFER_SIZE {
            buf.reserve(HW_BUFFER_SIZE - remaining);
        }

        match read(cx, io, buf) {
            Poll::Pending => {
                return if read_some { Ok(Some(false)) } else { Ok(None) };
            }
            Poll::Ready(Ok(n)) => {
                if n == 0 {
                    return Ok(Some(true));
                } else {
                    read_some = true;
                }
            }
            Poll::Ready(Err(e)) => {
                return if e.kind() == io::ErrorKind::WouldBlock {
                    if read_some {
                        Ok(Some(false))
                    } else {
                        Ok(None)
                    }
                } else if e.kind() == io::ErrorKind::ConnectionReset && read_some {
                    Ok(Some(true))
                } else {
                    Err(e)
                }
            }
        }
    }
}

fn read<T>(
    cx: &mut Context<'_>,
    io: &mut T,
    buf: &mut BytesMut,
) -> Poll<Result<usize, io::Error>>
where
    T: AsyncRead + Unpin,
{
    Pin::new(io).poll_read_buf(cx, buf)
}

#[cfg(test)]
mod tests {
    use actix_service::IntoService;
    use futures_util::future::{lazy, ok};

    use super::*;
    use crate::error::Error;
    use crate::h1::{ExpectHandler, UpgradeHandler};
    use crate::test::TestBuffer;

    #[actix_rt::test]
    async fn test_req_parse_err() {
        lazy(|cx| {
            let buf = TestBuffer::new("GET /test HTTP/1\r\n\r\n");

            let mut h1 = Dispatcher::<_, _, _, _, UpgradeHandler<TestBuffer>>::new(
                buf,
                ServiceConfig::default(),
                CloneableService::new(
                    (|_| ok::<_, Error>(Response::Ok().finish())).into_service(),
                ),
                CloneableService::new(ExpectHandler),
                None,
                None,
                Extensions::new(),
                None,
            );

            match Pin::new(&mut h1).poll(cx) {
                Poll::Pending => panic!(),
                Poll::Ready(res) => assert!(res.is_err()),
            }

            if let DispatcherState::Normal(ref mut inner) = h1.inner {
                assert!(inner.flags.contains(Flags::READ_DISCONNECT));
                assert_eq!(
                    &inner.io.take().unwrap().write_buf[..26],
                    b"HTTP/1.1 400 Bad Request\r\n"
                );
            }
        })
        .await;
    }
}
