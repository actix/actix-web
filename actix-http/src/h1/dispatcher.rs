use std::{
    collections::VecDeque,
    fmt,
    future::Future,
    io, mem, net,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use actix_codec::{AsyncRead, AsyncWrite, Decoder as _, Encoder as _, Framed, FramedParts};
use actix_rt::time::{sleep_until, Instant, Sleep};
use actix_service::Service;
use bitflags::bitflags;
use bytes::{Buf, BytesMut};
use futures_core::ready;
use log::error;
use pin_project_lite::pin_project;

use crate::{
    body::{BodySize, BoxBody, MessageBody},
    config::ServiceConfig,
    error::{DispatchError, ParseError, PayloadError},
    service::HttpFlow,
    Error, Extensions, OnConnectData, Request, Response, StatusCode,
};

use super::{
    codec::Codec,
    decoder::MAX_BUFFER_SIZE,
    payload::{Payload, PayloadSender, PayloadStatus},
    Message, MessageType,
};

const LW_BUFFER_SIZE: usize = 1024;
const HW_BUFFER_SIZE: usize = 1024 * 8;
const MAX_PIPELINED_MESSAGES: usize = 16;

bitflags! {
    pub struct Flags: u8 {
        /// Set when stream is read for first time.
        const STARTED          = 0b0000_0001;

        /// Set when full request-response cycle has occurred.
        const FINISHED         = 0b0000_0010;

        /// Set if connection is in keep-alive (inactive) state.
        const KEEP_ALIVE       = 0b0000_0100;

        /// Set if in shutdown procedure.
        const SHUTDOWN         = 0b0000_1000;

        /// Set if read-half is disconnected.
        const READ_DISCONNECT  = 0b0001_0000;

        /// Set if write-half is disconnected.
        const WRITE_DISCONNECT = 0b0010_0000;
    }
}

// there's 2 versions of Dispatcher state because of:
// https://github.com/taiki-e/pin-project-lite/issues/3
//
// tl;dr: pin-project-lite doesn't play well with other attribute macros

#[cfg(not(test))]
pin_project! {
    /// Dispatcher for HTTP/1.1 protocol
    pub struct Dispatcher<T, S, B, X, U>
    where
        S: Service<Request>,
        S::Error: Into<Response<BoxBody>>,

        B: MessageBody,

        X: Service<Request, Response = Request>,
        X::Error: Into<Response<BoxBody>>,

        U: Service<(Request, Framed<T, Codec>), Response = ()>,
        U::Error: fmt::Display,
    {
        #[pin]
        inner: DispatcherState<T, S, B, X, U>,
    }
}

#[cfg(test)]
pin_project! {
    /// Dispatcher for HTTP/1.1 protocol
    pub struct Dispatcher<T, S, B, X, U>
    where
        S: Service<Request>,
        S::Error: Into<Response<BoxBody>>,

        B: MessageBody,

        X: Service<Request, Response = Request>,
        X::Error: Into<Response<BoxBody>>,

        U: Service<(Request, Framed<T, Codec>), Response = ()>,
        U::Error: fmt::Display,
    {
        #[pin]
        pub(super) inner: DispatcherState<T, S, B, X, U>,

        // used in tests
        pub(super) poll_count: u64,
    }
}

pin_project! {
    #[project = DispatcherStateProj]
    pub(super) enum DispatcherState<T, S, B, X, U>
    where
        S: Service<Request>,
        S::Error: Into<Response<BoxBody>>,

        B: MessageBody,

        X: Service<Request, Response = Request>,
        X::Error: Into<Response<BoxBody>>,

        U: Service<(Request, Framed<T, Codec>), Response = ()>,
        U::Error: fmt::Display,
    {
        Normal { #[pin] inner: InnerDispatcher<T, S, B, X, U> },
        Upgrade { #[pin] fut: U::Future },
    }
}

pin_project! {
    #[project = InnerDispatcherProj]
    pub(super) struct InnerDispatcher<T, S, B, X, U>
    where
        S: Service<Request>,
        S::Error: Into<Response<BoxBody>>,

        B: MessageBody,

        X: Service<Request, Response = Request>,
        X::Error: Into<Response<BoxBody>>,

        U: Service<(Request, Framed<T, Codec>), Response = ()>,
        U::Error: fmt::Display,
    {
        flow: Rc<HttpFlow<S, X, U>>,
        pub(super) flags: Flags,
        peer_addr: Option<net::SocketAddr>,
        conn_data: Option<Rc<Extensions>>,
        config: ServiceConfig,
        error: Option<DispatchError>,

        #[pin]
        state: State<S, B, X>,
        payload: Option<PayloadSender>,
        messages: VecDeque<DispatcherMessage>,

        head_timer: TimerState,
        ka_timer: TimerState,
        shutdown_timer: TimerState,

        pub(super) io: Option<T>,
        read_buf: BytesMut,
        write_buf: BytesMut,
        codec: Codec,
    }
}

#[derive(Debug)]
enum TimerState {
    Disabled,
    Inactive,
    Active { timer: Pin<Box<Sleep>> },
}

impl TimerState {
    fn new(enabled: bool) -> Self {
        if enabled {
            Self::Inactive
        } else {
            Self::Disabled
        }
    }

    fn is_enabled(&self) -> bool {
        matches!(self, Self::Active { .. } | Self::Inactive)
    }

    fn set(&mut self, timer: Sleep, line: u32) {
        if !self.is_enabled() {
            warn!("setting disabled timer from line {}", line);
        }

        *self = Self::Active {
            timer: Box::pin(timer),
        };
    }

    fn clear(&mut self, line: u32) {
        if !self.is_enabled() {
            warn!("trying to clear a disabled timer from line {}", line);
        }

        if matches!(self, Self::Inactive) {
            warn!("trying to clear an inactive timer from line {}", line);
        }

        *self = Self::Inactive;
    }
}

impl fmt::Display for TimerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimerState::Disabled => f.write_str("timer is disabled"),
            TimerState::Inactive => f.write_str("timer is inactive"),
            TimerState::Active { timer } => {
                let deadline = timer.deadline();
                let now = Instant::now();

                if deadline < now {
                    f.write_str("timer is active and has reached deadline")
                } else {
                    write!(
                        f,
                        "timer is active and due to expire in {} milliseconds",
                        ((deadline - now).as_secs_f32() * 1000.0)
                    )
                }
            }
        }
    }
}

enum DispatcherMessage {
    Item(Request),
    Upgrade(Request),
    Error(Response<()>),
}

pin_project! {
    #[project = StateProj]
    enum State<S, B, X>
    where
        S: Service<Request>,
        X: Service<Request, Response = Request>,
        B: MessageBody,
    {
        None,
        ExpectCall { #[pin] fut: X::Future },
        ServiceCall { #[pin] fut: S::Future },
        SendPayload { #[pin] body: B },
        SendErrorPayload { #[pin] body: BoxBody },
    }
}

impl<S, B, X> State<S, B, X>
where
    S: Service<Request>,
    X: Service<Request, Response = Request>,
    B: MessageBody,
{
    fn is_none(&self) -> bool {
        matches!(self, State::None)
    }
}

impl<S, B, X> fmt::Debug for State<S, B, X>
where
    S: Service<Request>,
    X: Service<Request, Response = Request>,
    B: MessageBody,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => write!(f, "None"),
            Self::ExpectCall { .. } => f.debug_struct("ExpectCall").finish_non_exhaustive(),
            Self::ServiceCall { .. } => f.debug_struct("ServiceCall").finish_non_exhaustive(),
            Self::SendPayload { .. } => f.debug_struct("SendPayload").finish_non_exhaustive(),
            Self::SendErrorPayload { .. } => {
                f.debug_struct("SendErrorPayload").finish_non_exhaustive()
            }
        }
    }
}

#[derive(Debug)]
enum PollResponse {
    Upgrade(Request),
    DoNothing,
    DrainWriteBuf,
}

impl<T, S, B, X, U> Dispatcher<T, S, B, X, U>
where
    T: AsyncRead + AsyncWrite + Unpin,

    S: Service<Request>,
    S::Error: Into<Response<BoxBody>>,
    S::Response: Into<Response<B>>,

    B: MessageBody,

    X: Service<Request, Response = Request>,
    X::Error: Into<Response<BoxBody>>,

    U: Service<(Request, Framed<T, Codec>), Response = ()>,
    U::Error: fmt::Display,
{
    /// Create HTTP/1 dispatcher.
    pub(crate) fn new(
        io: T,
        flow: Rc<HttpFlow<S, X, U>>,
        config: ServiceConfig,
        peer_addr: Option<net::SocketAddr>,
        conn_data: OnConnectData,
    ) -> Self {
        Dispatcher {
            inner: DispatcherState::Normal {
                inner: InnerDispatcher {
                    flow,
                    flags: Flags::empty(),
                    peer_addr,
                    conn_data: conn_data.0.map(Rc::new),
                    config: config.clone(),
                    error: None,

                    state: State::None,
                    payload: None,
                    messages: VecDeque::new(),

                    head_timer: TimerState::new(config.client_request_deadline().is_some()),
                    ka_timer: TimerState::new(config.keep_alive_enabled()),
                    shutdown_timer: TimerState::new(
                        config.client_disconnect_deadline().is_some(),
                    ),

                    io: Some(io),
                    read_buf: BytesMut::with_capacity(HW_BUFFER_SIZE),
                    write_buf: BytesMut::with_capacity(HW_BUFFER_SIZE),
                    codec: Codec::new(config),
                },
            },

            #[cfg(test)]
            poll_count: 0,
        }
    }
}

impl<T, S, B, X, U> InnerDispatcher<T, S, B, X, U>
where
    T: AsyncRead + AsyncWrite + Unpin,

    S: Service<Request>,
    S::Error: Into<Response<BoxBody>>,
    S::Response: Into<Response<B>>,

    B: MessageBody,

    X: Service<Request, Response = Request>,
    X::Error: Into<Response<BoxBody>>,

    U: Service<(Request, Framed<T, Codec>), Response = ()>,
    U::Error: fmt::Display,
{
    fn can_read(&self, cx: &mut Context<'_>) -> bool {
        log::trace!("enter InnerDispatcher::can_read");

        if self.flags.contains(Flags::READ_DISCONNECT) {
            false
        } else if let Some(ref info) = self.payload {
            info.need_read(cx) == PayloadStatus::Read
        } else {
            true
        }
    }

    /// If checked is set to true, delay disconnect until all tasks have finished.
    fn client_disconnected(self: Pin<&mut Self>) {
        log::trace!("enter InnerDispatcher::client_disconnect");

        let this = self.project();

        this.flags
            .insert(Flags::READ_DISCONNECT | Flags::WRITE_DISCONNECT);

        if let Some(mut payload) = this.payload.take() {
            payload.set_error(PayloadError::Incomplete(None));
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        log::trace!("enter InnerDispatcher::poll_flush");

        let InnerDispatcherProj { io, write_buf, .. } = self.project();
        let mut io = Pin::new(io.as_mut().unwrap());

        let len = write_buf.len();
        let mut written = 0;

        while written < len {
            match io.as_mut().poll_write(cx, &write_buf[written..])? {
                Poll::Ready(0) => {
                    log::trace!("write zero error");
                    return Poll::Ready(Err(io::Error::new(io::ErrorKind::WriteZero, "")));
                }

                Poll::Ready(n) => written += n,

                Poll::Pending => {
                    write_buf.advance(written);
                    return Poll::Pending;
                }
            }
        }

        // everything has written to I/O; clear buffer
        write_buf.clear();

        // flush the I/O and check if get blocked
        io.poll_flush(cx)
    }

    fn send_response_inner(
        self: Pin<&mut Self>,
        res: Response<()>,
        body: &impl MessageBody,
    ) -> Result<BodySize, DispatchError> {
        log::trace!("enter InnerDispatcher::send_response_inner");

        let this = self.project();

        let size = body.size();

        this.codec
            .encode(Message::Item((res, size)), this.write_buf)
            .map_err(|err| {
                if let Some(mut payload) = this.payload.take() {
                    payload.set_error(PayloadError::Incomplete(None));
                }

                DispatchError::Io(err)
            })?;

        let conn_keep_alive = this.codec.keepalive();
        this.flags.set(Flags::KEEP_ALIVE, conn_keep_alive);

        if !conn_keep_alive {
            log::trace!("clearing keep-alive timer");
            this.ka_timer.clear(line!());
        }

        Ok(size)
    }

    fn send_response(
        mut self: Pin<&mut Self>,
        res: Response<()>,
        body: B,
    ) -> Result<(), DispatchError> {
        log::trace!("enter InnerDispatcher::send_response");

        let size = self.as_mut().send_response_inner(res, &body)?;
        let mut this = self.project();
        this.state.set(match size {
            BodySize::None | BodySize::Sized(0) => {
                this.flags.insert(Flags::FINISHED);
                State::None
            }
            _ => State::SendPayload { body },
        });

        Ok(())
    }

    fn send_error_response(
        mut self: Pin<&mut Self>,
        res: Response<()>,
        body: BoxBody,
    ) -> Result<(), DispatchError> {
        log::trace!("enter InnerDispatcher::send_error_response");

        let size = self.as_mut().send_response_inner(res, &body)?;
        let mut this = self.project();
        this.state.set(match size {
            BodySize::None | BodySize::Sized(0) => {
                this.flags.insert(Flags::FINISHED);
                State::None
            }
            _ => State::SendErrorPayload { body },
        });

        Ok(())
    }

    fn send_continue(self: Pin<&mut Self>) {
        log::trace!("enter InnerDispatcher::send_continue");

        self.project()
            .write_buf
            .extend_from_slice(b"HTTP/1.1 100 Continue\r\n\r\n");
    }

    fn poll_response(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Result<PollResponse, DispatchError> {
        'res: loop {
            log::trace!("enter InnerDispatcher::poll_response loop iteration");

            let mut this = self.as_mut().project();
            match this.state.as_mut().project() {
                // no future is in InnerDispatcher state; pop next message
                StateProj::None => match this.messages.pop_front() {
                    // handle request message
                    Some(DispatcherMessage::Item(req)) => {
                        // Handle `EXPECT: 100-Continue` header
                        if req.head().expect() {
                            log::trace!("  passing request to expect handler");
                            // set InnerDispatcher state and continue loop to poll it
                            let fut = this.flow.expect.call(req);
                            this.state.set(State::ExpectCall { fut });
                        } else {
                            log::trace!("  passing request to service handler");
                            // set InnerDispatcher state and continue loop to poll it
                            let fut = this.flow.service.call(req);
                            this.state.set(State::ServiceCall { fut });
                        };
                    }

                    // handle error message
                    Some(DispatcherMessage::Error(res)) => {
                        log::trace!("  handling dispatcher error message");
                        // send_response would update InnerDispatcher state to SendPayload or None
                        // (If response body is empty)
                        // continue loop to poll it
                        self.as_mut().send_error_response(res, BoxBody::new(()))?;
                    }

                    // return with upgrade request and poll it exclusively
                    Some(DispatcherMessage::Upgrade(req)) => {
                        // return upgrade
                        return Ok(PollResponse::Upgrade(req));
                    }

                    // all messages are dealt with
                    None => {
                        log::trace!("all messages handled");
                        return Ok(PollResponse::DoNothing);
                    }
                },

                StateProj::ServiceCall { fut } => {
                    log::trace!("  calling request handler service");

                    match fut.poll(cx) {
                        // service call resolved. send response.
                        Poll::Ready(Ok(res)) => {
                            log::trace!("  ok");
                            let (res, body) = res.into().replace_body(());
                            self.as_mut().send_response(res, body)?;
                        }

                        // send service call error as response
                        Poll::Ready(Err(err)) => {
                            log::trace!("  error");
                            let res: Response<BoxBody> = err.into();
                            let (res, body) = res.replace_body(());
                            self.as_mut().send_error_response(res, body)?;
                        }

                        // service call pending and could be waiting for more chunk messages
                        // (pipeline message limit and/or payload can_read limit)
                        Poll::Pending => {
                            log::trace!("  pending");
                            // no new message is decoded and no new payload is fed
                            // nothing to do except waiting for new incoming data from client
                            if !self.as_mut().poll_request(cx)? {
                                return Ok(PollResponse::DoNothing);
                            }
                            // else loop
                        }
                    }
                }

                StateProj::SendPayload { mut body } => {
                    log::trace!("sending payload");

                    // keep populate writer buffer until buffer size limit hit,
                    // get blocked or finished.
                    while this.write_buf.len() < super::payload::MAX_BUFFER_SIZE {
                        match body.as_mut().poll_next(cx) {
                            Poll::Ready(Some(Ok(item))) => {
                                this.codec
                                    .encode(Message::Chunk(Some(item)), this.write_buf)?;
                            }

                            Poll::Ready(None) => {
                                this.codec.encode(Message::Chunk(None), this.write_buf)?;

                                // payload stream finished.
                                // set state to None and handle next message
                                this.state.set(State::None);
                                this.flags.insert(Flags::FINISHED);

                                continue 'res;
                            }

                            Poll::Ready(Some(Err(err))) => {
                                this.flags.insert(Flags::FINISHED);
                                return Err(DispatchError::Body(err.into()));
                            }

                            Poll::Pending => return Ok(PollResponse::DoNothing),
                        }
                    }
                    // buffer is beyond max size.
                    // return and try to write the whole buffer to io stream.
                    return Ok(PollResponse::DrainWriteBuf);
                }

                StateProj::SendErrorPayload { mut body } => {
                    log::trace!("  sending error payload");

                    // TODO: de-dupe impl with SendPayload

                    // keep populate writer buffer until buffer size limit hit,
                    // get blocked or finished.
                    while this.write_buf.len() < super::payload::MAX_BUFFER_SIZE {
                        match body.as_mut().poll_next(cx) {
                            Poll::Ready(Some(Ok(item))) => {
                                this.codec
                                    .encode(Message::Chunk(Some(item)), this.write_buf)?;
                            }

                            Poll::Ready(None) => {
                                this.codec.encode(Message::Chunk(None), this.write_buf)?;
                                // payload stream finished
                                // set state to None and handle next message
                                this.state.set(State::None);
                                continue 'res;
                            }

                            Poll::Ready(Some(Err(err))) => {
                                return Err(DispatchError::Body(
                                    Error::new_body().with_cause(err).into(),
                                ))
                            }

                            Poll::Pending => return Ok(PollResponse::DoNothing),
                        }
                    }

                    // buffer is beyond max size
                    // return and try to write the whole buffer to stream
                    return Ok(PollResponse::DrainWriteBuf);
                }

                StateProj::ExpectCall { fut } => {
                    log::trace!("  calling expect service");

                    match fut.poll(cx) {
                        // expect resolved. write continue to buffer and set InnerDispatcher state
                        // to service call.
                        Poll::Ready(Ok(req)) => {
                            this.write_buf
                                .extend_from_slice(b"HTTP/1.1 100 Continue\r\n\r\n");
                            let fut = this.flow.service.call(req);
                            this.state.set(State::ServiceCall { fut });
                        }

                        // send expect error as response
                        Poll::Ready(Err(err)) => {
                            let res: Response<BoxBody> = err.into();
                            let (res, body) = res.replace_body(());
                            self.as_mut().send_error_response(res, body)?;
                        }

                        // expect must be solved before progress can be made.
                        Poll::Pending => return Ok(PollResponse::DoNothing),
                    }
                }
            }
        }
    }

    fn handle_request(
        mut self: Pin<&mut Self>,
        req: Request,
        cx: &mut Context<'_>,
    ) -> Result<(), DispatchError> {
        // initialize dispatcher state
        {
            let mut this = self.as_mut().project();

            // Handle `EXPECT: 100-Continue` header
            if req.head().expect() {
                // set dispatcher state to call expect handler
                let fut = this.flow.expect.call(req);
                this.state.set(State::ExpectCall { fut });
            } else {
                // set dispatcher state to call service handler
                let fut = this.flow.service.call(req);
                this.state.set(State::ServiceCall { fut });
            };
        };

        // eagerly poll the future once (or twice if expect is resolved immediately).
        loop {
            match self.as_mut().project().state.project() {
                StateProj::ExpectCall { fut } => {
                    match fut.poll(cx) {
                        // expect is resolved; continue loop and poll the service call branch.
                        Poll::Ready(Ok(req)) => {
                            self.as_mut().send_continue();

                            let mut this = self.as_mut().project();
                            let fut = this.flow.service.call(req);
                            this.state.set(State::ServiceCall { fut });

                            continue;
                        }

                        // future is error; send response and return a result
                        // on success to notify the dispatcher a new state is set and the outer loop
                        // should be continued
                        Poll::Ready(Err(err)) => {
                            let res: Response<BoxBody> = err.into();
                            let (res, body) = res.replace_body(());
                            return self.send_error_response(res, body);
                        }

                        // future is pending; return Ok(()) to notify that a new state is
                        // set and the outer loop should be continue.
                        Poll::Pending => return Ok(()),
                    }
                }

                StateProj::ServiceCall { fut } => {
                    // return no matter the service call future's result.
                    return match fut.poll(cx) {
                        // Future is resolved. Send response and return a result. On success
                        // to notify the dispatcher a new state is set and the outer loop
                        // should be continue.
                        Poll::Ready(Ok(res)) => {
                            let (res, body) = res.into().replace_body(());
                            self.as_mut().send_response(res, body)
                        }

                        // see the comment on ExpectCall state branch's Pending
                        Poll::Pending => Ok(()),

                        // see the comment on ExpectCall state branch's Ready(Err(_))
                        Poll::Ready(Err(err)) => {
                            let res: Response<BoxBody> = err.into();
                            let (res, body) = res.replace_body(());
                            self.as_mut().send_error_response(res, body)
                        }
                    };
                }

                _ => {
                    unreachable!(
                        "State must be set to ServiceCall or ExceptCall in handle_request"
                    )
                }
            }
        }
    }

    /// Process one incoming request.
    fn poll_request(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Result<bool, DispatchError> {
        log::trace!("enter InnerDispatcher::poll_request");

        let pipeline_queue_full = self.messages.len() >= MAX_PIPELINED_MESSAGES;
        let can_not_read = !self.can_read(cx);

        // limit amount of non-processed requests
        if pipeline_queue_full || can_not_read {
            return Ok(false);
        }

        let mut this = self.as_mut().project();

        loop {
            log::trace!("attempt to decode frame");

            match this.codec.decode(this.read_buf) {
                Ok(Some(msg)) => {
                    log::trace!("found full frame (head)");

                    match msg {
                        Message::Item(mut req) => {
                            this.head_timer.clear(line!());

                            req.head_mut().peer_addr = *this.peer_addr;

                            req.conn_data = this.conn_data.as_ref().map(Rc::clone);

                            match this.codec.message_type() {
                                // request has no payload
                                MessageType::None => {}

                                // Request is upgradable. Add upgrade message and break.
                                // Everything remaining in read buffer will be handed to
                                // upgraded Request.
                                MessageType::Stream if this.flow.upgrade.is_some() => {
                                    this.messages.push_back(DispatcherMessage::Upgrade(req));
                                    break;
                                }

                                // request is not upgradable
                                MessageType::Payload | MessageType::Stream => {
                                    // PayloadSender and Payload are smart pointers share the
                                    // same state. PayloadSender is attached to dispatcher and used
                                    // to sink new chunked request data to state. Payload is
                                    // attached to Request and passed to Service::call where the
                                    // state can be collected and consumed.
                                    let (sender, payload) = Payload::create(false);
                                    *req.payload() = crate::Payload::H1 { payload };
                                    *this.payload = Some(sender);
                                }
                            }

                            // handle request early when no future in InnerDispatcher state.
                            if this.state.is_none() {
                                self.as_mut().handle_request(req, cx)?;
                                this = self.as_mut().project();
                            } else {
                                this.messages.push_back(DispatcherMessage::Item(req));
                            }
                        }

                        Message::Chunk(Some(chunk)) => {
                            if let Some(ref mut payload) = this.payload {
                                payload.feed_data(chunk);
                            } else {
                                error!("Internal server error: unexpected payload chunk");
                                this.flags.insert(Flags::READ_DISCONNECT);
                                this.messages.push_back(DispatcherMessage::Error(
                                    Response::internal_server_error().drop_body(),
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
                                    Response::internal_server_error().drop_body(),
                                ));
                                *this.error = Some(DispatchError::InternalError);
                                break;
                            }
                        }
                    }
                }

                // decode is partial and buffer is not full yet
                // break and wait for more read
                Ok(None) => {
                    log::trace!("found partial frame");
                    break;
                }

                Err(ParseError::Io(err)) => {
                    log::trace!("io error: {}", &err);
                    self.as_mut().client_disconnected();
                    this = self.as_mut().project();
                    *this.error = Some(DispatchError::Io(err));
                    break;
                }

                Err(ParseError::TooLarge) => {
                    log::trace!("request head is too big");

                    if let Some(mut payload) = this.payload.take() {
                        payload.set_error(PayloadError::Overflow);
                    }

                    // request heads that overflow buffer size return a 431 error
                    this.messages
                        .push_back(DispatcherMessage::Error(Response::with_body(
                            StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
                            (),
                        )));

                    this.flags.insert(Flags::READ_DISCONNECT);
                    *this.error = Some(ParseError::TooLarge.into());

                    break;
                }

                Err(err) => {
                    log::trace!("parse error {}", &err);

                    if let Some(mut payload) = this.payload.take() {
                        payload.set_error(PayloadError::EncodingCorrupted);
                    }

                    // malformed requests should be responded with 400
                    this.messages.push_back(DispatcherMessage::Error(
                        Response::bad_request().drop_body(),
                    ));

                    this.flags.insert(Flags::READ_DISCONNECT);
                    *this.error = Some(err.into());
                    break;
                }
            }
        }

        // TODO: what's this boolean do now?
        Ok(false)
    }

    fn poll_head_timer(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Result<(), DispatchError> {
        let this = self.as_mut().project();

        match this.head_timer {
            TimerState::Active { timer } => {
                if timer.as_mut().poll(cx).is_ready() {
                    // timeout on first request (slow request) return 408

                    log::trace!(
                        "timed out on slow request; \
                        replying with 408 and closing connection"
                    );

                    let _ = self.as_mut().send_error_response(
                        Response::with_body(StatusCode::REQUEST_TIMEOUT, ()),
                        BoxBody::new(()),
                    );

                    self.project().flags.insert(Flags::SHUTDOWN);
                }
            }
            TimerState::Inactive => {}
            TimerState::Disabled => {}
        };

        Ok(())
    }

    fn poll_ka_timer(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Result<(), DispatchError> {
        let this = self.as_mut().project();
        match this.ka_timer {
            TimerState::Active { timer } => {
                debug_assert!(
                    this.flags.contains(Flags::KEEP_ALIVE),
                    "keep-alive flag should be set when timer is active",
                );
                debug_assert!(
                    this.state.is_none(),
                    "dispatcher should not be in keep-alive phase if state is not none",
                );
                debug_assert!(
                    this.write_buf.is_empty(),
                    "dispatcher should not be in keep-alive phase if write_buf is not empty",
                );

                // keep-alive timer has timed out
                if timer.as_mut().poll(cx).is_ready() {
                    // no tasks at hand
                    log::trace!("timer timed out; closing connection");
                    this.flags.insert(Flags::SHUTDOWN);

                    if let Some(deadline) = this.config.client_disconnect_deadline() {
                        // start shutdown timeout if enabled
                        log::trace!("starting disconnect timer");
                        this.shutdown_timer.set(sleep_until(deadline), line!());
                    } else {
                        // no shutdown timeout, drop socket
                        this.flags.insert(Flags::WRITE_DISCONNECT);
                    }
                }
            }
            TimerState::Disabled => {}
            TimerState::Inactive => {}
        }

        Ok(())
    }

    fn poll_shutdown_timer(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Result<(), DispatchError> {
        let this = self.as_mut().project();
        match this.shutdown_timer {
            TimerState::Disabled => {}
            TimerState::Inactive => {}
            TimerState::Active { timer } => {
                debug_assert!(
                    this.flags.contains(Flags::SHUTDOWN),
                    "shutdown flag should be set when timer is active",
                );

                // timed-out during shutdown; drop connection
                if timer.as_mut().poll(cx).is_ready() {
                    log::trace!("timed-out during shutdown");
                    return Err(DispatchError::DisconnectTimeout);
                }

                // if this.flags.contains(Flags::SHUTDOWN) {
                //     log::trace!("start shutdown timer");

                //     if let Some(deadline) = this.config.client_disconnect_deadline() {
                //         // write client disconnect time out and poll again to
                //         // go into Some(timer) branch
                //         this.timer.set(Some(sleep_until(deadline)));
                //         return self.poll_timer(cx);
                //     }
                // }
            }
        }

        Ok(())
    }

    /// Poll head, keep-alive, and disconnect timer.
    fn poll_timer(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Result<(), DispatchError> {
        log::trace!("enter InnerDispatcher::poll_timer");
        trace_timer_states(&self.head_timer, &self.ka_timer, &self.shutdown_timer);

        self.as_mut().poll_head_timer(cx)?;
        self.as_mut().poll_ka_timer(cx)?;
        self.as_mut().poll_shutdown_timer(cx)?;

        Ok(())
    }

    /// Returns true when I/O stream can be disconnected after write to it.
    ///
    /// It covers these conditions:
    /// - `std::io::ErrorKind::ConnectionReset` after partial read;
    /// - all data read done.
    #[inline(always)] // TODO: bench this inline
    fn read_available(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Result<bool, DispatchError> {
        log::trace!("enter InnerDispatcher::read_available");
        log::trace!("  reading from a {}", core::any::type_name::<T>());

        let this = self.project();

        if this.flags.contains(Flags::READ_DISCONNECT) {
            log::trace!("  read DC");
            return Ok(false);
        };

        let mut io = Pin::new(this.io.as_mut().unwrap());

        let mut read_some = false;

        loop {
            // Return early when read buf exceed decoder's max buffer size.
            if this.read_buf.len() >= MAX_BUFFER_SIZE {
                // At this point it's not known IO stream is still scheduled to be waked up so
                // force wake up dispatcher just in case.
                //
                // Reason:
                // AsyncRead mostly would only have guarantee wake up when the poll_read
                // return Poll::Pending.
                //
                // Case:
                // When read_buf is beyond max buffer size the early return could be successfully
                // be parsed as a new Request. This case would not generate ParseError::TooLarge and
                // at this point IO stream is not fully read to Pending and would result in
                // dispatcher stuck until timeout (keep-alive).
                //
                // Note:
                // This is a perf choice to reduce branch on <Request as MessageType>::decode.
                //
                // A Request head too large to parse is only checked on `httparse::Status::Partial`.

                if this.payload.is_none() {
                    // When dispatcher has a payload the responsibility of wake up it would be shift
                    // to h1::payload::Payload.
                    //
                    // Reason:
                    // Self wake up when there is payload would waste poll and/or result in
                    // over read.
                    //
                    // Case:
                    // When payload is (partial) dropped by user there is no need to do
                    // read anymore. At this case read_buf could always remain beyond
                    // MAX_BUFFER_SIZE and self wake up would be busy poll dispatcher and
                    // waste resources.
                    cx.waker().wake_by_ref();
                }

                return Ok(false);
            }

            // grow buffer if necessary.
            let remaining = this.read_buf.capacity() - this.read_buf.len();
            if remaining < LW_BUFFER_SIZE {
                this.read_buf.reserve(HW_BUFFER_SIZE - remaining);
            }

            match actix_codec::poll_read_buf(io.as_mut(), cx, this.read_buf) {
                Poll::Ready(Ok(n)) => {
                    log::trace!("  read {} bytes", n);

                    if n == 0 {
                        log::trace!("  signalling should_disconnect");
                        return Ok(true);
                    }

                    read_some = true;
                }
                Poll::Pending => {
                    log::trace!("  read pending");
                    return Ok(false);
                }
                Poll::Ready(Err(err)) => {
                    log::trace!("  read err: {:?}", &err);

                    return match err.kind() {
                        // convert WouldBlock error to the same as Pending return
                        io::ErrorKind::WouldBlock => Ok(false),

                        // connection reset after partial read
                        io::ErrorKind::ConnectionReset if read_some => Ok(true),

                        _ => Err(DispatchError::Io(err)),
                    };
                }
            }
        }
    }

    /// call upgrade service with request.
    fn upgrade(self: Pin<&mut Self>, req: Request) -> U::Future {
        let this = self.project();
        let mut parts = FramedParts::with_read_buf(
            this.io.take().unwrap(),
            mem::take(this.codec),
            mem::take(this.read_buf),
        );
        parts.write_buf = mem::take(this.write_buf);
        let framed = Framed::from_parts(parts);
        this.flow.upgrade.as_ref().unwrap().call((req, framed))
    }
}

impl<T, S, B, X, U> Future for Dispatcher<T, S, B, X, U>
where
    T: AsyncRead + AsyncWrite + Unpin,

    S: Service<Request>,
    S::Error: Into<Response<BoxBody>>,
    S::Response: Into<Response<B>>,

    B: MessageBody,

    X: Service<Request, Response = Request>,
    X::Error: Into<Response<BoxBody>>,

    U: Service<(Request, Framed<T, Codec>), Response = ()>,
    U::Error: fmt::Display,
{
    type Output = Result<(), DispatchError>;

    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        log::trace!(target: "", "");
        log::trace!("enter Dispatcher::poll");

        let this = self.as_mut().project();

        #[cfg(test)]
        {
            *this.poll_count += 1;
        }

        match this.inner.project() {
            DispatcherStateProj::Normal { mut inner } => {
                log::trace!("current flags: {:?}", &inner.flags);

                inner.as_mut().poll_timer(cx)?;

                let poll = if inner.flags.contains(Flags::SHUTDOWN) {
                    if inner.flags.contains(Flags::WRITE_DISCONNECT) {
                        Poll::Ready(Ok(()))
                    } else {
                        // flush buffer and wait on blocked
                        ready!(inner.as_mut().poll_flush(cx))?;
                        Pin::new(inner.as_mut().project().io.as_mut().unwrap())
                            .poll_shutdown(cx)
                            .map_err(DispatchError::from)
                    }
                } else {
                    // read from I/O stream and fill read buffer
                    let should_disconnect = inner.as_mut().read_available(cx)?;

                    if !inner.flags.contains(Flags::STARTED) {
                        log::trace!("set started flag");
                        inner.as_mut().project().flags.insert(Flags::STARTED);

                        if let Some(deadline) = inner.config.client_request_deadline() {
                            log::trace!("start head timer");
                            inner
                                .as_mut()
                                .project()
                                .head_timer
                                .set(sleep_until(deadline), line!());
                        }
                    }

                    inner.as_mut().poll_request(cx)?;

                    if should_disconnect {
                        log::trace!("should_disconnect = true");
                        // I/O stream should to be closed
                        let inner = inner.as_mut().project();
                        inner.flags.insert(Flags::READ_DISCONNECT);
                        if let Some(mut payload) = inner.payload.take() {
                            payload.feed_eof();
                        }
                    };

                    loop {
                        // poll response to populate write buffer
                        // drain indicates whether write buffer should be emptied before next run
                        let drain = match inner.as_mut().poll_response(cx)? {
                            PollResponse::DrainWriteBuf => {
                                inner.flags.contains(Flags::KEEP_ALIVE);
                                true
                            }

                            PollResponse::DoNothing => {
                                if inner.flags.contains(Flags::KEEP_ALIVE) {
                                    if let Some(deadline) = inner.config.keep_alive_timer() {
                                        log::trace!("setting keep-alive timer");
                                        inner
                                            .as_mut()
                                            .project()
                                            .ka_timer
                                            .set(deadline, line!());
                                    }
                                }

                                false
                            }

                            // upgrade request and goes Upgrade variant of DispatcherState.
                            PollResponse::Upgrade(req) => {
                                let upgrade = inner.upgrade(req);
                                self.as_mut()
                                    .project()
                                    .inner
                                    .set(DispatcherState::Upgrade { fut: upgrade });
                                return self.poll(cx);
                            }
                        };

                        // we didn't get WouldBlock from write operation,
                        // so data get written to kernel completely (macOS)
                        // and we have to write again otherwise response can get stuck
                        //
                        // TODO: what? is WouldBlock good or bad?
                        // want to find a reference for this macOS behavior
                        if inner.as_mut().poll_flush(cx)?.is_pending() || !drain {
                            log::trace!("break out of poll_response loop after poll_flush");
                            break;
                        }
                    }

                    // client is gone
                    if inner.flags.contains(Flags::WRITE_DISCONNECT) {
                        log::trace!("client is gone; disconnecting");
                        return Poll::Ready(Ok(()));
                    }

                    let inner_p = inner.as_mut().project();
                    let state_is_none = inner_p.state.is_none();

                    // read half is closed; we do not process any responses
                    if inner_p.flags.contains(Flags::READ_DISCONNECT) && state_is_none {
                        log::trace!("read half closed; start shutdown");
                        inner_p.flags.insert(Flags::SHUTDOWN);
                    }

                    // keep-alive and stream errors
                    if state_is_none && inner_p.write_buf.is_empty() {
                        log::trace!("state is None and write buf is empty");

                        if let Some(err) = inner_p.error.take() {
                            log::trace!("stream error {}", &err);
                            return Poll::Ready(Err(err));
                        }

                        // disconnect if keep-alive is not enabled
                        if inner_p.flags.contains(Flags::FINISHED)
                            && !inner_p.flags.contains(Flags::KEEP_ALIVE)
                        {
                            log::trace!(
                                "start shutdown because keep-alive is disabled or opted \
                                out for this connection"
                            );
                            inner_p.flags.insert(Flags::SHUTDOWN);
                            return self.poll(cx);
                        }

                        // disconnect if shutdown
                        if inner_p.flags.contains(Flags::SHUTDOWN) {
                            log::trace!("shutdown from shutdown flag");
                            return self.poll(cx);
                        }
                    }

                    log::trace!("dispatcher going to sleep; wait for next event");

                    trace_timer_states(
                        inner_p.head_timer,
                        inner_p.ka_timer,
                        inner_p.shutdown_timer,
                    );

                    Poll::Pending
                };

                log::trace!("current flags: {:?}", &inner.flags);

                poll
            }

            DispatcherStateProj::Upgrade { fut: upgrade } => upgrade.poll(cx).map_err(|err| {
                error!("Upgrade handler error: {}", err);
                DispatchError::Upgrade
            }),
        }
    }
}

fn trace_timer_states(
    head_timer: &TimerState,
    ka_timer: &TimerState,
    shutdown_timer: &TimerState,
) {
    log::trace!("timers:");

    if head_timer.is_enabled() {
        log::trace!("  head {}", &head_timer);
    }

    if ka_timer.is_enabled() {
        log::trace!("  keep-alive {}", &ka_timer);
    }

    if shutdown_timer.is_enabled() {
        log::trace!("  shutdown {}", &shutdown_timer);
    }
}
