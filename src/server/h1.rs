use std::collections::VecDeque;
use std::net::{Shutdown, SocketAddr};
use std::time::{Duration, Instant};

use bytes::BytesMut;
use futures::{Async, Future, Poll};
use tokio_current_thread::spawn;
use tokio_timer::Delay;

use error::{Error, PayloadError};
use http::{StatusCode, Version};
use payload::{Payload, PayloadStatus, PayloadWriter};

use super::error::{HttpDispatchError, ServerError};
use super::h1decoder::{DecoderError, H1Decoder, Message};
use super::h1writer::H1Writer;
use super::handler::{HttpHandler, HttpHandlerTask, HttpHandlerTaskFut};
use super::input::PayloadType;
use super::settings::ServiceConfig;
use super::{IoStream, Writer};

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
pub struct Http1Dispatcher<T: IoStream, H: HttpHandler + 'static> {
    flags: Flags,
    settings: ServiceConfig<H>,
    addr: Option<SocketAddr>,
    stream: H1Writer<T, H>,
    decoder: H1Decoder,
    payload: Option<PayloadType>,
    buf: BytesMut,
    tasks: VecDeque<Entry<H>>,
    error: Option<HttpDispatchError>,
    ka_expire: Instant,
    ka_timer: Option<Delay>,
}

enum Entry<H: HttpHandler> {
    Task(H::Task),
    Error(Box<HttpHandlerTask>),
}

impl<H: HttpHandler> Entry<H> {
    fn into_task(self) -> H::Task {
        match self {
            Entry::Task(task) => task,
            Entry::Error(_) => panic!(),
        }
    }
    fn disconnected(&mut self) {
        match *self {
            Entry::Task(ref mut task) => task.disconnected(),
            Entry::Error(ref mut task) => task.disconnected(),
        }
    }
    fn poll_io(&mut self, io: &mut Writer) -> Poll<bool, Error> {
        match *self {
            Entry::Task(ref mut task) => task.poll_io(io),
            Entry::Error(ref mut task) => task.poll_io(io),
        }
    }
    fn poll_completed(&mut self) -> Poll<(), Error> {
        match *self {
            Entry::Task(ref mut task) => task.poll_completed(),
            Entry::Error(ref mut task) => task.poll_completed(),
        }
    }
}

impl<T, H> Http1Dispatcher<T, H>
where
    T: IoStream,
    H: HttpHandler + 'static,
{
    pub fn new(
        settings: ServiceConfig<H>, stream: T, buf: BytesMut, is_eof: bool,
        keepalive_timer: Option<Delay>,
    ) -> Self {
        let addr = stream.peer_addr();
        let (ka_expire, ka_timer) = if let Some(delay) = keepalive_timer {
            (delay.deadline(), Some(delay))
        } else if let Some(delay) = settings.keep_alive_timer() {
            (delay.deadline(), Some(delay))
        } else {
            (settings.now(), None)
        };

        let flags = if is_eof {
            Flags::READ_DISCONNECTED | Flags::FLUSHED
        } else if settings.keep_alive_enabled() {
            Flags::KEEPALIVE | Flags::KEEPALIVE_ENABLED | Flags::FLUSHED
        } else {
            Flags::empty()
        };

        Http1Dispatcher {
            stream: H1Writer::new(stream, settings.clone()),
            decoder: H1Decoder::new(),
            payload: None,
            tasks: VecDeque::new(),
            error: None,
            flags,
            addr,
            buf,
            settings,
            ka_timer,
            ka_expire,
        }
    }

    pub(crate) fn for_error(
        settings: ServiceConfig<H>, stream: T, status: StatusCode,
        mut keepalive_timer: Option<Delay>, buf: BytesMut,
    ) -> Self {
        if let Some(deadline) = settings.client_timer_expire() {
            let _ = keepalive_timer.as_mut().map(|delay| delay.reset(deadline));
        }

        let mut disp = Http1Dispatcher {
            flags: Flags::STARTED | Flags::READ_DISCONNECTED | Flags::FLUSHED,
            stream: H1Writer::new(stream, settings.clone()),
            decoder: H1Decoder::new(),
            payload: None,
            tasks: VecDeque::new(),
            error: None,
            addr: None,
            ka_timer: keepalive_timer,
            ka_expire: settings.now(),
            buf,
            settings,
        };
        disp.push_response_entry(status);
        disp
    }

    #[inline]
    pub fn settings(&self) -> &ServiceConfig<H> {
        &self.settings
    }

    #[inline]
    pub(crate) fn io(&mut self) -> &mut T {
        self.stream.get_mut()
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

        if !checked || self.tasks.is_empty() {
            self.flags
                .insert(Flags::WRITE_DISCONNECTED | Flags::FLUSHED);
            self.stream.disconnected();

            // notify all tasks
            for mut task in self.tasks.drain(..) {
                task.disconnected();
                match task.poll_completed() {
                    Ok(Async::NotReady) => {
                        // spawn not completed task, it does not require access to io
                        // at this point
                        spawn(HttpHandlerTaskFut::new(task.into_task()));
                    }
                    Ok(Async::Ready(_)) => (),
                    Err(err) => {
                        error!("Unhandled application error: {}", err);
                    }
                }
            }
        }
    }

    #[inline]
    pub fn poll(&mut self) -> Poll<(), HttpDispatchError> {
        // check connection keep-alive
        self.poll_keepalive()?;

        // shutdown
        if self.flags.contains(Flags::SHUTDOWN) {
            if self.flags.contains(Flags::WRITE_DISCONNECTED) {
                return Ok(Async::Ready(()));
            }
            return self.poll_flush(true);
        }

        // process incoming requests
        if !self.flags.contains(Flags::WRITE_DISCONNECTED) {
            self.poll_handler()?;

            // flush stream
            self.poll_flush(false)?;

            // deal with keep-alive and stream eof (client-side write shutdown)
            if self.tasks.is_empty() && self.flags.contains(Flags::FLUSHED) {
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

    /// Flush stream
    fn poll_flush(&mut self, shutdown: bool) -> Poll<(), HttpDispatchError> {
        if shutdown || self.flags.contains(Flags::STARTED) {
            match self.stream.poll_completed(shutdown) {
                Ok(Async::NotReady) => {
                    // mark stream
                    if !self.stream.flushed() {
                        self.flags.remove(Flags::FLUSHED);
                    }
                    Ok(Async::NotReady)
                }
                Err(err) => {
                    debug!("Error sending data: {}", err);
                    self.client_disconnected(false);
                    Err(err.into())
                }
                Ok(Async::Ready(_)) => {
                    // if payload is not consumed we can not use connection
                    if self.payload.is_some() && self.tasks.is_empty() {
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

    /// keep-alive timer. returns `true` is keep-alive, otherwise drop
    fn poll_keepalive(&mut self) -> Result<(), HttpDispatchError> {
        if let Some(ref mut timer) = self.ka_timer {
            match timer.poll() {
                Ok(Async::Ready(_)) => {
                    // if we get timer during shutdown, just drop connection
                    if self.flags.contains(Flags::SHUTDOWN) {
                        let io = self.stream.get_mut();
                        let _ = IoStream::set_linger(io, Some(Duration::from_secs(0)));
                        let _ = IoStream::shutdown(io, Shutdown::Both);
                        return Err(HttpDispatchError::ShutdownTimeout);
                    }
                    if timer.deadline() >= self.ka_expire {
                        // check for any outstanding request handling
                        if self.tasks.is_empty() {
                            if !self.flags.contains(Flags::STARTED) {
                                // timeout on first request (slow request) return 408
                                trace!("Slow request timeout");
                                self.flags
                                    .insert(Flags::STARTED | Flags::READ_DISCONNECTED);
                                self.tasks.push_back(Entry::Error(ServerError::err(
                                    Version::HTTP_11,
                                    StatusCode::REQUEST_TIMEOUT,
                                )));
                            } else {
                                trace!("Keep-alive timeout, close connection");
                                self.flags.insert(Flags::SHUTDOWN);

                                // start shutdown timer
                                if let Some(deadline) =
                                    self.settings.client_shutdown_timer()
                                {
                                    timer.reset(deadline)
                                } else {
                                    return Ok(());
                                }
                            }
                        } else if let Some(dl) = self.settings.keep_alive_expire() {
                            timer.reset(dl)
                        }
                    } else {
                        timer.reset(self.ka_expire)
                    }
                }
                Ok(Async::NotReady) => (),
                Err(e) => {
                    error!("Timer error {:?}", e);
                    return Err(HttpDispatchError::Unknown);
                }
            }
        }

        Ok(())
    }

    #[inline]
    /// read data from the stream
    pub(self) fn poll_io(&mut self) -> Result<bool, HttpDispatchError> {
        if !self.flags.contains(Flags::POLLED) {
            self.flags.insert(Flags::POLLED);
            if !self.buf.is_empty() {
                let updated = self.parse()?;
                return Ok(updated);
            }
        }

        // read io from socket
        let mut updated = false;
        if self.can_read() && self.tasks.len() < MAX_PIPELINED_MESSAGES {
            match self.stream.get_mut().read_available(&mut self.buf) {
                Ok(Async::Ready((read_some, disconnected))) => {
                    if read_some && self.parse()? {
                        updated = true;
                    }
                    if disconnected {
                        self.client_disconnected(true);
                    }
                }
                Ok(Async::NotReady) => (),
                Err(err) => {
                    self.client_disconnected(false);
                    return Err(err.into());
                }
            }
        }
        Ok(updated)
    }

    pub(self) fn poll_handler(&mut self) -> Result<(), HttpDispatchError> {
        self.poll_io()?;
        let mut retry = self.can_read();

        // process first pipelined response, only first task can do io operation in http/1
        while !self.tasks.is_empty() {
            match self.tasks[0].poll_io(&mut self.stream) {
                Ok(Async::Ready(ready)) => {
                    // override keep-alive state
                    if self.stream.keepalive() {
                        self.flags.insert(Flags::KEEPALIVE);
                    } else {
                        self.flags.remove(Flags::KEEPALIVE);
                    }
                    // prepare stream for next response
                    self.stream.reset();

                    let task = self.tasks.pop_front().unwrap();
                    if !ready {
                        // task is done with io operations but still needs to do more work
                        spawn(HttpHandlerTaskFut::new(task.into_task()));
                    }
                }
                Ok(Async::NotReady) => {
                    // check if we need timer
                    if self.ka_timer.is_some() && self.stream.upgrade() {
                        self.ka_timer.take();
                    }

                    // if read-backpressure is enabled and we consumed some data.
                    // we may read more dataand retry
                    if !retry && self.can_read() && self.poll_io()? {
                        retry = self.can_read();
                        continue;
                    }
                    break;
                }
                Err(err) => {
                    error!("Unhandled error1: {}", err);
                    // it is not possible to recover from error
                    // during pipe handling, so just drop connection
                    self.client_disconnected(false);
                    return Err(err.into());
                }
            }
        }

        // check in-flight messages. all tasks must be alive,
        // they need to produce response. if app returned error
        // and we can not continue processing incoming requests.
        let mut idx = 1;
        while idx < self.tasks.len() {
            let stop = match self.tasks[idx].poll_completed() {
                Ok(Async::NotReady) => false,
                Ok(Async::Ready(_)) => true,
                Err(err) => {
                    self.error = Some(err.into());
                    true
                }
            };
            if stop {
                // error in task handling or task is completed,
                // so no response for this task which means we can not read more requests
                // because pipeline sequence is broken.
                // but we can safely complete existing tasks
                self.flags.insert(Flags::READ_DISCONNECTED);

                for mut task in self.tasks.drain(idx..) {
                    task.disconnected();
                    match task.poll_completed() {
                        Ok(Async::NotReady) => {
                            // spawn not completed task, it does not require access to io
                            // at this point
                            spawn(HttpHandlerTaskFut::new(task.into_task()));
                        }
                        Ok(Async::Ready(_)) => (),
                        Err(err) => {
                            error!("Unhandled application error: {}", err);
                        }
                    }
                }
                break;
            } else {
                idx += 1;
            }
        }

        Ok(())
    }

    fn push_response_entry(&mut self, status: StatusCode) {
        self.tasks
            .push_back(Entry::Error(ServerError::err(Version::HTTP_11, status)));
    }

    pub(self) fn parse(&mut self) -> Result<bool, HttpDispatchError> {
        let mut updated = false;

        'outer: loop {
            match self.decoder.decode(&mut self.buf, &self.settings) {
                Ok(Some(Message::Message { mut msg, payload })) => {
                    updated = true;
                    self.flags.insert(Flags::STARTED);

                    if payload {
                        let (ps, pl) = Payload::new(false);
                        *msg.inner.payload.borrow_mut() = Some(pl);
                        self.payload = Some(PayloadType::new(&msg.inner.headers, ps));
                    }

                    // stream extensions
                    msg.inner_mut().stream_extensions =
                        self.stream.get_mut().extensions();

                    // set remote addr
                    msg.inner_mut().addr = self.addr;

                    // search handler for request
                    match self.settings.handler().handle(msg) {
                        Ok(mut task) => {
                            if self.tasks.is_empty() {
                                match task.poll_io(&mut self.stream) {
                                    Ok(Async::Ready(ready)) => {
                                        // override keep-alive state
                                        if self.stream.keepalive() {
                                            self.flags.insert(Flags::KEEPALIVE);
                                        } else {
                                            self.flags.remove(Flags::KEEPALIVE);
                                        }
                                        // prepare stream for next response
                                        self.stream.reset();

                                        if !ready {
                                            // task is done with io operations
                                            // but still needs to do more work
                                            spawn(HttpHandlerTaskFut::new(task));
                                        }
                                        continue 'outer;
                                    }
                                    Ok(Async::NotReady) => (),
                                    Err(err) => {
                                        error!("Unhandled error: {}", err);
                                        self.client_disconnected(false);
                                        return Err(err.into());
                                    }
                                }
                            }
                            self.tasks.push_back(Entry::Task(task));
                            continue 'outer;
                        }
                        Err(_) => {
                            // handler is not found
                            self.push_response_entry(StatusCode::NOT_FOUND);
                        }
                    }
                }
                Ok(Some(Message::Chunk(chunk))) => {
                    updated = true;
                    if let Some(ref mut payload) = self.payload {
                        payload.feed_data(chunk);
                    } else {
                        error!("Internal server error: unexpected payload chunk");
                        self.flags.insert(Flags::READ_DISCONNECTED | Flags::STARTED);
                        self.push_response_entry(StatusCode::INTERNAL_SERVER_ERROR);
                        self.error = Some(HttpDispatchError::InternalError);
                        break;
                    }
                }
                Ok(Some(Message::Eof)) => {
                    updated = true;
                    if let Some(mut payload) = self.payload.take() {
                        payload.feed_eof();
                    } else {
                        error!("Internal server error: unexpected eof");
                        self.flags.insert(Flags::READ_DISCONNECTED | Flags::STARTED);
                        self.push_response_entry(StatusCode::INTERNAL_SERVER_ERROR);
                        self.error = Some(HttpDispatchError::InternalError);
                        break;
                    }
                }
                Ok(None) => {
                    if self.flags.contains(Flags::READ_DISCONNECTED) {
                        self.client_disconnected(true);
                    }
                    break;
                }
                Err(e) => {
                    if let Some(mut payload) = self.payload.take() {
                        let e = match e {
                            DecoderError::Io(e) => PayloadError::Io(e),
                            DecoderError::Error(_) => PayloadError::EncodingCorrupted,
                        };
                        payload.set_error(e);
                    }

                    // Malformed requests should be responded with 400
                    self.push_response_entry(StatusCode::BAD_REQUEST);
                    self.flags.insert(Flags::READ_DISCONNECTED | Flags::STARTED);
                    self.error = Some(HttpDispatchError::MalformedRequest);
                    break;
                }
            }
        }

        if self.ka_timer.is_some() && updated {
            if let Some(expire) = self.settings.keep_alive_expire() {
                self.ka_expire = expire;
            }
        }
        Ok(updated)
    }
}

#[cfg(test)]
mod tests {
    use std::net::Shutdown;
    use std::{cmp, io, time};

    use actix::System;
    use bytes::{Buf, Bytes, BytesMut};
    use futures::future;
    use http::{Method, Version};
    use tokio_io::{AsyncRead, AsyncWrite};

    use super::*;
    use application::{App, HttpApplication};
    use httpmessage::HttpMessage;
    use server::h1decoder::Message;
    use server::handler::IntoHttpHandler;
    use server::settings::{ServerSettings, ServiceConfig};
    use server::{KeepAlive, Request};

    fn wrk_settings() -> ServiceConfig<HttpApplication> {
        ServiceConfig::<HttpApplication>::new(
            App::new().into_handler(),
            KeepAlive::Os,
            5000,
            2000,
            ServerSettings::default(),
        )
    }

    impl Message {
        fn message(self) -> Request {
            match self {
                Message::Message { msg, payload: _ } => msg,
                _ => panic!("error"),
            }
        }
        fn is_payload(&self) -> bool {
            match *self {
                Message::Message { msg: _, payload } => payload,
                _ => panic!("error"),
            }
        }
        fn chunk(self) -> Bytes {
            match self {
                Message::Chunk(chunk) => chunk,
                _ => panic!("error"),
            }
        }
        fn eof(&self) -> bool {
            match *self {
                Message::Eof => true,
                _ => false,
            }
        }
    }

    macro_rules! parse_ready {
        ($e:expr) => {{
            let settings = wrk_settings();
            match H1Decoder::new().decode($e, &settings) {
                Ok(Some(msg)) => msg.message(),
                Ok(_) => unreachable!("Eof during parsing http request"),
                Err(err) => unreachable!("Error during parsing http request: {:?}", err),
            }
        }};
    }

    macro_rules! expect_parse_err {
        ($e:expr) => {{
            let settings = wrk_settings();

            match H1Decoder::new().decode($e, &settings) {
                Err(err) => match err {
                    DecoderError::Error(_) => (),
                    _ => unreachable!("Parse error expected"),
                },
                _ => unreachable!("Error expected"),
            }
        }};
    }

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

    impl IoStream for Buffer {
        fn shutdown(&mut self, _: Shutdown) -> io::Result<()> {
            Ok(())
        }
        fn set_nodelay(&mut self, _: bool) -> io::Result<()> {
            Ok(())
        }
        fn set_linger(&mut self, _: Option<time::Duration>) -> io::Result<()> {
            Ok(())
        }
        fn set_keepalive(&mut self, _: Option<time::Duration>) -> io::Result<()> {
            Ok(())
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
        let mut sys = System::new("test");
        let _ = sys.block_on(future::lazy(|| {
            let buf = Buffer::new("GET /test HTTP/1\r\n\r\n");
            let readbuf = BytesMut::new();
            let settings = wrk_settings();

            let mut h1 =
                Http1Dispatcher::new(settings.clone(), buf, readbuf, false, None);
            assert!(h1.poll_io().is_ok());
            assert!(h1.poll_io().is_ok());
            assert!(h1.flags.contains(Flags::READ_DISCONNECTED));
            assert_eq!(h1.tasks.len(), 1);
            future::ok::<_, ()>(())
        }));
    }

    #[test]
    fn test_parse() {
        let mut buf = BytesMut::from("GET /test HTTP/1.1\r\n\r\n");
        let settings = wrk_settings();

        let mut reader = H1Decoder::new();
        match reader.decode(&mut buf, &settings) {
            Ok(Some(msg)) => {
                let req = msg.message();
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
            }
            Ok(_) | Err(_) => unreachable!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_partial() {
        let mut buf = BytesMut::from("PUT /test HTTP/1");
        let settings = wrk_settings();

        let mut reader = H1Decoder::new();
        match reader.decode(&mut buf, &settings) {
            Ok(None) => (),
            _ => unreachable!("Error"),
        }

        buf.extend(b".1\r\n\r\n");
        match reader.decode(&mut buf, &settings) {
            Ok(Some(msg)) => {
                let mut req = msg.message();
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::PUT);
                assert_eq!(req.path(), "/test");
            }
            Ok(_) | Err(_) => unreachable!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_post() {
        let mut buf = BytesMut::from("POST /test2 HTTP/1.0\r\n\r\n");
        let settings = wrk_settings();

        let mut reader = H1Decoder::new();
        match reader.decode(&mut buf, &settings) {
            Ok(Some(msg)) => {
                let mut req = msg.message();
                assert_eq!(req.version(), Version::HTTP_10);
                assert_eq!(*req.method(), Method::POST);
                assert_eq!(req.path(), "/test2");
            }
            Ok(_) | Err(_) => unreachable!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_body() {
        let mut buf =
            BytesMut::from("GET /test HTTP/1.1\r\nContent-Length: 4\r\n\r\nbody");
        let settings = wrk_settings();

        let mut reader = H1Decoder::new();
        match reader.decode(&mut buf, &settings) {
            Ok(Some(msg)) => {
                let mut req = msg.message();
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
                assert_eq!(
                    reader
                        .decode(&mut buf, &settings)
                        .unwrap()
                        .unwrap()
                        .chunk()
                        .as_ref(),
                    b"body"
                );
            }
            Ok(_) | Err(_) => unreachable!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_body_crlf() {
        let mut buf =
            BytesMut::from("\r\nGET /test HTTP/1.1\r\nContent-Length: 4\r\n\r\nbody");
        let settings = wrk_settings();

        let mut reader = H1Decoder::new();
        match reader.decode(&mut buf, &settings) {
            Ok(Some(msg)) => {
                let mut req = msg.message();
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
                assert_eq!(
                    reader
                        .decode(&mut buf, &settings)
                        .unwrap()
                        .unwrap()
                        .chunk()
                        .as_ref(),
                    b"body"
                );
            }
            Ok(_) | Err(_) => unreachable!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_partial_eof() {
        let mut buf = BytesMut::from("GET /test HTTP/1.1\r\n");
        let settings = wrk_settings();
        let mut reader = H1Decoder::new();
        assert!(reader.decode(&mut buf, &settings).unwrap().is_none());

        buf.extend(b"\r\n");
        match reader.decode(&mut buf, &settings) {
            Ok(Some(msg)) => {
                let req = msg.message();
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
            }
            Ok(_) | Err(_) => unreachable!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_headers_split_field() {
        let mut buf = BytesMut::from("GET /test HTTP/1.1\r\n");
        let settings = wrk_settings();

        let mut reader = H1Decoder::new();
        assert!{ reader.decode(&mut buf, &settings).unwrap().is_none() }

        buf.extend(b"t");
        assert!{ reader.decode(&mut buf, &settings).unwrap().is_none() }

        buf.extend(b"es");
        assert!{ reader.decode(&mut buf, &settings).unwrap().is_none() }

        buf.extend(b"t: value\r\n\r\n");
        match reader.decode(&mut buf, &settings) {
            Ok(Some(msg)) => {
                let req = msg.message();
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
                assert_eq!(req.headers().get("test").unwrap().as_bytes(), b"value");
            }
            Ok(_) | Err(_) => unreachable!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_headers_multi_value() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             Set-Cookie: c1=cookie1\r\n\
             Set-Cookie: c2=cookie2\r\n\r\n",
        );
        let settings = wrk_settings();
        let mut reader = H1Decoder::new();
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        let req = msg.message();

        let val: Vec<_> = req
            .headers()
            .get_all("Set-Cookie")
            .iter()
            .map(|v| v.to_str().unwrap().to_owned())
            .collect();
        assert_eq!(val[0], "c1=cookie1");
        assert_eq!(val[1], "c2=cookie2");
    }

    #[test]
    fn test_conn_default_1_0() {
        let mut buf = BytesMut::from("GET /test HTTP/1.0\r\n\r\n");
        let req = parse_ready!(&mut buf);

        assert!(!req.keep_alive());
    }

    #[test]
    fn test_conn_default_1_1() {
        let mut buf = BytesMut::from("GET /test HTTP/1.1\r\n\r\n");
        let req = parse_ready!(&mut buf);

        assert!(req.keep_alive());
    }

    #[test]
    fn test_conn_close() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             connection: close\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(!req.keep_alive());
    }

    #[test]
    fn test_conn_close_1_0() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.0\r\n\
             connection: close\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(!req.keep_alive());
    }

    #[test]
    fn test_conn_keep_alive_1_0() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.0\r\n\
             connection: keep-alive\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(req.keep_alive());
    }

    #[test]
    fn test_conn_keep_alive_1_1() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             connection: keep-alive\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(req.keep_alive());
    }

    #[test]
    fn test_conn_other_1_0() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.0\r\n\
             connection: other\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(!req.keep_alive());
    }

    #[test]
    fn test_conn_other_1_1() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             connection: other\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(req.keep_alive());
    }

    #[test]
    fn test_conn_upgrade() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             upgrade: websockets\r\n\
             connection: upgrade\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(req.upgrade());
    }

    #[test]
    fn test_conn_upgrade_connect_method() {
        let mut buf = BytesMut::from(
            "CONNECT /test HTTP/1.1\r\n\
             content-type: text/plain\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(req.upgrade());
    }

    #[test]
    fn test_request_chunked() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        if let Ok(val) = req.chunked() {
            assert!(val);
        } else {
            unreachable!("Error");
        }

        // type in chunked
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chnked\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        if let Ok(val) = req.chunked() {
            assert!(!val);
        } else {
            unreachable!("Error");
        }
    }

    #[test]
    fn test_headers_content_length_err_1() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             content-length: line\r\n\r\n",
        );

        expect_parse_err!(&mut buf)
    }

    #[test]
    fn test_headers_content_length_err_2() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             content-length: -1\r\n\r\n",
        );

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_invalid_header() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             test line\r\n\r\n",
        );

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_invalid_name() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             test[]: line\r\n\r\n",
        );

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_http_request_bad_status_line() {
        let mut buf = BytesMut::from("getpath \r\n\r\n");
        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_http_request_upgrade() {
        let settings = wrk_settings();
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             connection: upgrade\r\n\
             upgrade: websocket\r\n\r\n\
             some raw data",
        );
        let mut reader = H1Decoder::new();
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert!(msg.is_payload());
        let req = msg.message();
        assert!(!req.keep_alive());
        assert!(req.upgrade());
        assert_eq!(
            reader
                .decode(&mut buf, &settings)
                .unwrap()
                .unwrap()
                .chunk()
                .as_ref(),
            b"some raw data"
        );
    }

    #[test]
    fn test_http_request_parser_utf8() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             x-test: тест\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert_eq!(
            req.headers().get("x-test").unwrap().as_bytes(),
            "тест".as_bytes()
        );
    }

    #[test]
    fn test_http_request_parser_two_slashes() {
        let mut buf = BytesMut::from("GET //path HTTP/1.1\r\n\r\n");
        let req = parse_ready!(&mut buf);

        assert_eq!(req.path(), "//path");
    }

    #[test]
    fn test_http_request_parser_bad_method() {
        let mut buf = BytesMut::from("!12%()+=~$ /get HTTP/1.1\r\n\r\n");

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_http_request_parser_bad_version() {
        let mut buf = BytesMut::from("GET //get HT/11\r\n\r\n");

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_http_request_chunked_payload() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n",
        );
        let settings = wrk_settings();
        let mut reader = H1Decoder::new();
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert!(msg.is_payload());
        let req = msg.message();
        assert!(req.chunked().unwrap());

        buf.extend(b"4\r\ndata\r\n4\r\nline\r\n0\r\n\r\n");
        assert_eq!(
            reader
                .decode(&mut buf, &settings)
                .unwrap()
                .unwrap()
                .chunk()
                .as_ref(),
            b"data"
        );
        assert_eq!(
            reader
                .decode(&mut buf, &settings)
                .unwrap()
                .unwrap()
                .chunk()
                .as_ref(),
            b"line"
        );
        assert!(reader.decode(&mut buf, &settings).unwrap().unwrap().eof());
    }

    #[test]
    fn test_http_request_chunked_payload_and_next_message() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n",
        );
        let settings = wrk_settings();
        let mut reader = H1Decoder::new();
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert!(msg.is_payload());
        let req = msg.message();
        assert!(req.chunked().unwrap());

        buf.extend(
            b"4\r\ndata\r\n4\r\nline\r\n0\r\n\r\n\
              POST /test2 HTTP/1.1\r\n\
              transfer-encoding: chunked\r\n\r\n"
                .iter(),
        );
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert_eq!(msg.chunk().as_ref(), b"data");
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert_eq!(msg.chunk().as_ref(), b"line");
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert!(msg.eof());

        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert!(msg.is_payload());
        let req2 = msg.message();
        assert!(req2.chunked().unwrap());
        assert_eq!(*req2.method(), Method::POST);
        assert!(req2.chunked().unwrap());
    }

    #[test]
    fn test_http_request_chunked_payload_chunks() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n",
        );
        let settings = wrk_settings();

        let mut reader = H1Decoder::new();
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert!(msg.is_payload());
        let req = msg.message();
        assert!(req.chunked().unwrap());

        buf.extend(b"4\r\n1111\r\n");
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert_eq!(msg.chunk().as_ref(), b"1111");

        buf.extend(b"4\r\ndata\r");
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert_eq!(msg.chunk().as_ref(), b"data");

        buf.extend(b"\n4");
        assert!(reader.decode(&mut buf, &settings).unwrap().is_none());

        buf.extend(b"\r");
        assert!(reader.decode(&mut buf, &settings).unwrap().is_none());
        buf.extend(b"\n");
        assert!(reader.decode(&mut buf, &settings).unwrap().is_none());

        buf.extend(b"li");
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert_eq!(msg.chunk().as_ref(), b"li");

        //trailers
        //buf.feed_data("test: test\r\n");
        //not_ready!(reader.parse(&mut buf, &mut readbuf));

        buf.extend(b"ne\r\n0\r\n");
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert_eq!(msg.chunk().as_ref(), b"ne");
        assert!(reader.decode(&mut buf, &settings).unwrap().is_none());

        buf.extend(b"\r\n");
        assert!(reader.decode(&mut buf, &settings).unwrap().unwrap().eof());
    }

    #[test]
    fn test_parse_chunked_payload_chunk_extension() {
        let mut buf = BytesMut::from(
            &"GET /test HTTP/1.1\r\n\
              transfer-encoding: chunked\r\n\r\n"[..],
        );
        let settings = wrk_settings();

        let mut reader = H1Decoder::new();
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert!(msg.is_payload());
        assert!(msg.message().chunked().unwrap());

        buf.extend(b"4;test\r\ndata\r\n4\r\nline\r\n0\r\n\r\n"); // test: test\r\n\r\n")
        let chunk = reader.decode(&mut buf, &settings).unwrap().unwrap().chunk();
        assert_eq!(chunk, Bytes::from_static(b"data"));
        let chunk = reader.decode(&mut buf, &settings).unwrap().unwrap().chunk();
        assert_eq!(chunk, Bytes::from_static(b"line"));
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert!(msg.eof());
    }
}
