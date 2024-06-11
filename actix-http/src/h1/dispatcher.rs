use std::{
    collections::VecDeque,
    fmt,
    future::Future,
    io, mem, net,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use actix_codec::{Framed, FramedParts};
use actix_rt::time::sleep_until;
use actix_service::Service;
use bitflags::bitflags;
use bytes::{Buf, BytesMut};
use futures_core::ready;
use pin_project_lite::pin_project;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{Decoder as _, Encoder as _};
use tracing::{error, trace};

use super::{
    codec::Codec,
    decoder::MAX_BUFFER_SIZE,
    payload::{Payload, PayloadSender, PayloadStatus},
    timer::TimerState,
    Message, MessageType,
};
use crate::{
    body::{BodySize, BoxBody, MessageBody},
    config::ServiceConfig,
    error::{DispatchError, ParseError, PayloadError},
    service::HttpFlow,
    Error, Extensions, OnConnectData, Request, Response, StatusCode,
};

const LW_BUFFER_SIZE: usize = 1024;
const HW_BUFFER_SIZE: usize = 1024 * 8;
const MAX_PIPELINED_MESSAGES: usize = 16;

bitflags! {
    #[derive(Debug, Clone, Copy)]
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
        pub(super) state: State<S, B, X>,
        // when Some(_) dispatcher is in state of receiving request payload
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

enum DispatcherMessage {
    Item(Request),
    Upgrade(Request),
    Error(Response<()>),
}

pin_project! {
    #[project = StateProj]
    pub(super) enum State<S, B, X>
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
    pub(super) fn is_none(&self) -> bool {
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
            Self::None => write!(f, "State::None"),
            Self::ExpectCall { .. } => f.debug_struct("State::ExpectCall").finish_non_exhaustive(),
            Self::ServiceCall { .. } => {
                f.debug_struct("State::ServiceCall").finish_non_exhaustive()
            }
            Self::SendPayload { .. } => {
                f.debug_struct("State::SendPayload").finish_non_exhaustive()
            }
            Self::SendErrorPayload { .. } => f
                .debug_struct("State::SendErrorPayload")
                .finish_non_exhaustive(),
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
                    ka_timer: TimerState::new(config.keep_alive().enabled()),
                    shutdown_timer: TimerState::new(config.client_disconnect_deadline().is_some()),

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
        if self.flags.contains(Flags::READ_DISCONNECT) {
            false
        } else if let Some(ref info) = self.payload {
            info.need_read(cx) == PayloadStatus::Read
        } else {
            true
        }
    }

    fn client_disconnected(self: Pin<&mut Self>) {
        let this = self.project();

        this.flags
            .insert(Flags::READ_DISCONNECT | Flags::WRITE_DISCONNECT);

        if let Some(mut payload) = this.payload.take() {
            payload.set_error(PayloadError::Incomplete(None));
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        let InnerDispatcherProj { io, write_buf, .. } = self.project();
        let mut io = Pin::new(io.as_mut().unwrap());

        let len = write_buf.len();
        let mut written = 0;

        while written < len {
            match io.as_mut().poll_write(cx, &write_buf[written..])? {
                Poll::Ready(0) => {
                    error!("write zero; closing");
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

        Ok(size)
    }

    fn send_response(
        mut self: Pin<&mut Self>,
        res: Response<()>,
        body: B,
    ) -> Result<(), DispatchError> {
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
        self.project()
            .write_buf
            .extend_from_slice(b"HTTP/1.1 100 Continue\r\n\r\n");
    }

    fn poll_response(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Result<PollResponse, DispatchError> {
        'res: loop {
            let mut this = self.as_mut().project();
            match this.state.as_mut().project() {
                // no future is in InnerDispatcher state; pop next message
                StateProj::None => match this.messages.pop_front() {
                    // handle request message
                    Some(DispatcherMessage::Item(req)) => {
                        // Handle `EXPECT: 100-Continue` header
                        if req.head().expect() {
                            // set InnerDispatcher state and continue loop to poll it
                            let fut = this.flow.expect.call(req);
                            this.state.set(State::ExpectCall { fut });
                        } else {
                            // set InnerDispatcher state and continue loop to poll it
                            let fut = this.flow.service.call(req);
                            this.state.set(State::ServiceCall { fut });
                        };
                    }

                    // handle error message
                    Some(DispatcherMessage::Error(res)) => {
                        // send_response would update InnerDispatcher state to SendPayload or None
                        // (If response body is empty)
                        // continue loop to poll it
                        self.as_mut().send_error_response(res, BoxBody::new(()))?;
                    }

                    // return with upgrade request and poll it exclusively
                    Some(DispatcherMessage::Upgrade(req)) => return Ok(PollResponse::Upgrade(req)),

                    // all messages are dealt with
                    None => {
                        // start keep-alive if last request allowed it
                        this.flags.set(Flags::KEEP_ALIVE, this.codec.keep_alive());

                        return Ok(PollResponse::DoNothing);
                    }
                },

                StateProj::ServiceCall { fut } => {
                    match fut.poll(cx) {
                        // service call resolved. send response.
                        Poll::Ready(Ok(res)) => {
                            let (res, body) = res.into().replace_body(());
                            self.as_mut().send_response(res, body)?;
                        }

                        // send service call error as response
                        Poll::Ready(Err(err)) => {
                            let res: Response<BoxBody> = err.into();
                            let (res, body) = res.replace_body(());
                            self.as_mut().send_error_response(res, body)?;
                        }

                        // service call pending and could be waiting for more chunk messages
                        // (pipeline message limit and/or payload can_read limit)
                        Poll::Pending => {
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
                                let err = err.into();
                                tracing::error!("Response payload stream error: {err:?}");
                                this.flags.insert(Flags::FINISHED);
                                return Err(DispatchError::Body(err));
                            }

                            Poll::Pending => return Ok(PollResponse::DoNothing),
                        }
                    }

                    // buffer is beyond max size
                    // return and try to write the whole buffer to I/O stream.
                    return Ok(PollResponse::DrainWriteBuf);
                }

                StateProj::SendErrorPayload { mut body } => {
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
                                this.flags.insert(Flags::FINISHED);

                                continue 'res;
                            }

                            Poll::Ready(Some(Err(err))) => {
                                tracing::error!("Response payload stream error: {err:?}");
                                this.flags.insert(Flags::FINISHED);
                                return Err(DispatchError::Body(
                                    Error::new_body().with_cause(err).into(),
                                ));
                            }

                            Poll::Pending => return Ok(PollResponse::DoNothing),
                        }
                    }

                    // buffer is beyond max size
                    // return and try to write the whole buffer to stream
                    return Ok(PollResponse::DrainWriteBuf);
                }

                StateProj::ExpectCall { fut } => {
                    trace!("  calling expect service");

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
                    unreachable!("State must be set to ServiceCall or ExceptCall in handle_request")
                }
            }
        }
    }

    /// Process one incoming request.
    ///
    /// Returns true if any meaningful work was done.
    fn poll_request(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Result<bool, DispatchError> {
        let pipeline_queue_full = self.messages.len() >= MAX_PIPELINED_MESSAGES;
        let can_not_read = !self.can_read(cx);

        // limit amount of non-processed requests
        if pipeline_queue_full || can_not_read {
            return Ok(false);
        }

        let mut this = self.as_mut().project();

        let mut updated = false;

        // decode from read buf as many full requests as possible
        loop {
            match this.codec.decode(this.read_buf) {
                Ok(Some(msg)) => {
                    updated = true;

                    match msg {
                        Message::Item(mut req) => {
                            // head timer only applies to first request on connection
                            this.head_timer.clear(line!());

                            req.head_mut().peer_addr = *this.peer_addr;

                            req.conn_data.clone_from(this.conn_data);

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
                Ok(None) => break,

                Err(ParseError::Io(err)) => {
                    trace!("I/O error: {}", &err);
                    self.as_mut().client_disconnected();
                    this = self.as_mut().project();
                    *this.error = Some(DispatchError::Io(err));
                    break;
                }

                Err(ParseError::TooLarge) => {
                    trace!("request head was too big; returning 431 response");

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
                    trace!("parse error {}", &err);

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

        Ok(updated)
    }

    fn poll_head_timer(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Result<(), DispatchError> {
        let this = self.as_mut().project();

        if let TimerState::Active { timer } = this.head_timer {
            if timer.as_mut().poll(cx).is_ready() {
                // timeout on first request (slow request) return 408

                trace!("timed out on slow request; replying with 408 and closing connection");

                let _ = self.as_mut().send_error_response(
                    Response::with_body(StatusCode::REQUEST_TIMEOUT, ()),
                    BoxBody::new(()),
                );

                self.project().flags.insert(Flags::SHUTDOWN);
            }
        };

        Ok(())
    }

    fn poll_ka_timer(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Result<(), DispatchError> {
        let this = self.as_mut().project();
        if let TimerState::Active { timer } = this.ka_timer {
            debug_assert!(
                this.flags.contains(Flags::KEEP_ALIVE),
                "keep-alive flag should be set when timer is active",
            );
            debug_assert!(
                this.state.is_none(),
                "dispatcher should not be in keep-alive phase if state is not none: {:?}",
                this.state,
            );

            // Assert removed by @robjtede on account of issue #2655. There are cases where an I/O
            // flush can be pending after entering the keep-alive state causing the subsequent flush
            // wake up to panic here. This appears to be a Linux-only problem. Leaving original code
            // below for posterity because a simple and reliable test could not be found to trigger
            // the behavior.
            // debug_assert!(
            //     this.write_buf.is_empty(),
            //     "dispatcher should not be in keep-alive phase if write_buf is not empty",
            // );

            // keep-alive timer has timed out
            if timer.as_mut().poll(cx).is_ready() {
                // no tasks at hand
                trace!("timer timed out; closing connection");
                this.flags.insert(Flags::SHUTDOWN);

                if let Some(deadline) = this.config.client_disconnect_deadline() {
                    // start shutdown timeout if enabled
                    this.shutdown_timer
                        .set_and_init(cx, sleep_until(deadline.into()), line!());
                } else {
                    // no shutdown timeout, drop socket
                    this.flags.insert(Flags::WRITE_DISCONNECT);
                }
            }
        }

        Ok(())
    }

    fn poll_shutdown_timer(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Result<(), DispatchError> {
        let this = self.as_mut().project();
        if let TimerState::Active { timer } = this.shutdown_timer {
            debug_assert!(
                this.flags.contains(Flags::SHUTDOWN),
                "shutdown flag should be set when timer is active",
            );

            // timed-out during shutdown; drop connection
            if timer.as_mut().poll(cx).is_ready() {
                trace!("timed-out during shutdown");
                return Err(DispatchError::DisconnectTimeout);
            }
        }

        Ok(())
    }

    /// Poll head, keep-alive, and disconnect timer.
    fn poll_timers(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Result<(), DispatchError> {
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
    fn read_available(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Result<bool, DispatchError> {
        let this = self.project();

        if this.flags.contains(Flags::READ_DISCONNECT) {
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

                match this.payload {
                    // When dispatcher has a payload the responsibility of wake ups is shifted to
                    // `h1::payload::Payload` unless the payload is needing a read, in which case it
                    // might not have access to the waker and could result in the dispatcher
                    // getting stuck until timeout.
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
                    Some(ref p) if p.need_read(cx) != PayloadStatus::Read => {}
                    _ => cx.waker().wake_by_ref(),
                }

                return Ok(false);
            }

            // grow buffer if necessary.
            let remaining = this.read_buf.capacity() - this.read_buf.len();
            if remaining < LW_BUFFER_SIZE {
                this.read_buf.reserve(HW_BUFFER_SIZE - remaining);
            }

            match tokio_util::io::poll_read_buf(io.as_mut(), cx, this.read_buf) {
                Poll::Ready(Ok(n)) => {
                    this.flags.remove(Flags::FINISHED);

                    if n == 0 {
                        return Ok(true);
                    }

                    read_some = true;
                }

                Poll::Pending => {
                    return Ok(false);
                }

                Poll::Ready(Err(err)) => {
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
        let this = self.as_mut().project();

        #[cfg(test)]
        {
            *this.poll_count += 1;
        }

        match this.inner.project() {
            DispatcherStateProj::Upgrade { fut: upgrade } => upgrade.poll(cx).map_err(|err| {
                error!("Upgrade handler error: {}", err);
                DispatchError::Upgrade
            }),

            DispatcherStateProj::Normal { mut inner } => {
                trace!("start flags: {:?}", &inner.flags);

                trace_timer_states(
                    "start",
                    &inner.head_timer,
                    &inner.ka_timer,
                    &inner.shutdown_timer,
                );

                inner.as_mut().poll_timers(cx)?;

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

                    // after reading something from stream, clear keep-alive timer
                    if !inner.read_buf.is_empty() && inner.flags.contains(Flags::KEEP_ALIVE) {
                        let inner = inner.as_mut().project();
                        inner.flags.remove(Flags::KEEP_ALIVE);
                        inner.ka_timer.clear(line!());
                    }

                    if !inner.flags.contains(Flags::STARTED) {
                        inner.as_mut().project().flags.insert(Flags::STARTED);

                        if let Some(deadline) = inner.config.client_request_deadline() {
                            inner.as_mut().project().head_timer.set_and_init(
                                cx,
                                sleep_until(deadline.into()),
                                line!(),
                            );
                        }
                    }

                    inner.as_mut().poll_request(cx)?;

                    if should_disconnect {
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
                            PollResponse::DrainWriteBuf => true,

                            PollResponse::DoNothing => {
                                // KEEP_ALIVE is set in send_response_inner if client allows it
                                // FINISHED is set after writing last chunk of response
                                if inner.flags.contains(Flags::KEEP_ALIVE | Flags::FINISHED) {
                                    if let Some(timer) = inner.config.keep_alive_deadline() {
                                        inner.as_mut().project().ka_timer.set_and_init(
                                            cx,
                                            sleep_until(timer.into()),
                                            line!(),
                                        );
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

                        // we didn't get WouldBlock from write operation, so data get written to
                        // kernel completely (macOS) and we have to write again otherwise response
                        // can get stuck
                        //
                        // TODO: want to find a reference for this behavior
                        // see introduced commit: 3872d3ba
                        let flush_was_ready = inner.as_mut().poll_flush(cx)?.is_ready();

                        // this assert seems to always be true but not willing to commit to it until
                        // we understand what Nikolay meant when writing the above comment
                        // debug_assert!(flush_was_ready);

                        if !flush_was_ready || !drain {
                            break;
                        }
                    }

                    // client is gone
                    if inner.flags.contains(Flags::WRITE_DISCONNECT) {
                        trace!("client is gone; disconnecting");
                        return Poll::Ready(Ok(()));
                    }

                    let inner_p = inner.as_mut().project();
                    let state_is_none = inner_p.state.is_none();

                    // read half is closed; we do not process any responses
                    if inner_p.flags.contains(Flags::READ_DISCONNECT) && state_is_none {
                        trace!("read half closed; start shutdown");
                        inner_p.flags.insert(Flags::SHUTDOWN);
                    }

                    // keep-alive and stream errors
                    if state_is_none && inner_p.write_buf.is_empty() {
                        if let Some(err) = inner_p.error.take() {
                            error!("stream error: {}", &err);
                            return Poll::Ready(Err(err));
                        }

                        // disconnect if keep-alive is not enabled
                        if inner_p.flags.contains(Flags::FINISHED)
                            && !inner_p.flags.contains(Flags::KEEP_ALIVE)
                        {
                            inner_p.flags.remove(Flags::FINISHED);
                            inner_p.flags.insert(Flags::SHUTDOWN);
                            return self.poll(cx);
                        }

                        // disconnect if shutdown
                        if inner_p.flags.contains(Flags::SHUTDOWN) {
                            return self.poll(cx);
                        }
                    }

                    trace_timer_states(
                        "end",
                        inner_p.head_timer,
                        inner_p.ka_timer,
                        inner_p.shutdown_timer,
                    );

                    Poll::Pending
                };

                trace!("end flags: {:?}", &inner.flags);

                poll
            }
        }
    }
}

#[allow(dead_code)]
fn trace_timer_states(
    label: &str,
    head_timer: &TimerState,
    ka_timer: &TimerState,
    shutdown_timer: &TimerState,
) {
    trace!("{} timers:", label);

    if head_timer.is_enabled() {
        trace!("  head {}", &head_timer);
    }

    if ka_timer.is_enabled() {
        trace!("  keep-alive {}", &ka_timer);
    }

    if shutdown_timer.is_enabled() {
        trace!("  shutdown {}", &shutdown_timer);
    }
}
