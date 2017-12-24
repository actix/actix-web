use std::{self, io};
use std::rc::Rc;
use std::net::SocketAddr;
use std::time::Duration;
use std::collections::VecDeque;

use actix::Arbiter;
use httparse;
use http::{Uri, Method, Version, HttpTryFrom, HeaderMap};
use http::header::{self, HeaderName, HeaderValue};
use bytes::{Bytes, BytesMut, BufMut};
use futures::{Future, Poll, Async};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_core::reactor::Timeout;

use pipeline::Pipeline;
use encoding::PayloadType;
use channel::{HttpHandler, HttpHandlerTask};
use h1writer::{Writer, H1Writer};
use server::WorkerSettings;
use httpcodes::HTTPNotFound;
use httprequest::HttpRequest;
use error::{ParseError, PayloadError, ResponseError};
use payload::{Payload, PayloadWriter, DEFAULT_BUFFER_SIZE};

const LW_BUFFER_SIZE: usize = 4096;
const HW_BUFFER_SIZE: usize = 16_384;
const MAX_BUFFER_SIZE: usize = 131_072;
const MAX_HEADERS: usize = 100;
const MAX_PIPELINED_MESSAGES: usize = 16;
const HTTP2_PREFACE: [u8; 14] = *b"PRI * HTTP/2.0";

bitflags! {
    struct Flags: u8 {
        const ERROR = 0b0000_0010;
        const KEEPALIVE = 0b0000_0100;
        const H2 = 0b0000_1000;
    }
}

bitflags! {
    struct EntryFlags: u8 {
        const EOF = 0b0000_0001;
        const ERROR = 0b0000_0010;
        const FINISHED = 0b0000_0100;
    }
}

pub(crate) enum Http1Result {
    Done,
    Switch,
}

#[derive(Debug)]
enum Item {
    Http1(HttpRequest),
    Http2,
}

pub(crate) struct Http1<T: AsyncWrite + 'static, H: 'static> {
    flags: Flags,
    settings: Rc<WorkerSettings<H>>,
    addr: Option<SocketAddr>,
    stream: H1Writer<T>,
    reader: Reader,
    read_buf: BytesMut,
    tasks: VecDeque<Entry>,
    keepalive_timer: Option<Timeout>,
}

struct Entry {
    pipe: Box<HttpHandlerTask>,
    flags: EntryFlags,
}

impl<T, H> Http1<T, H>
    where T: AsyncRead + AsyncWrite + 'static,
          H: HttpHandler + 'static
{
    pub fn new(h: Rc<WorkerSettings<H>>, stream: T, addr: Option<SocketAddr>) -> Self {
        let bytes = h.get_shared_bytes();
        Http1{ flags: Flags::KEEPALIVE,
               settings: h,
               addr: addr,
               stream: H1Writer::new(stream, bytes),
               reader: Reader::new(),
               read_buf: BytesMut::new(),
               tasks: VecDeque::new(),
               keepalive_timer: None }
    }

    pub fn into_inner(self) -> (Rc<WorkerSettings<H>>, T, Option<SocketAddr>, Bytes) {
        (self.settings, self.stream.into_inner(), self.addr, self.read_buf.freeze())
    }

    fn poll_completed(&mut self) -> Result<bool, ()> {
        // check stream state
        match self.stream.poll_completed() {
            Ok(Async::Ready(_)) => Ok(false),
            Ok(Async::NotReady) => Ok(true),
            Err(err) => {
                debug!("Error sending data: {}", err);
                Err(())
            }
        }
    }

    // TODO: refacrtor
    #[cfg_attr(feature = "cargo-clippy", allow(cyclomatic_complexity))]
    pub fn poll(&mut self) -> Poll<Http1Result, ()> {
        // keep-alive timer
        if self.keepalive_timer.is_some() {
            match self.keepalive_timer.as_mut().unwrap().poll() {
                Ok(Async::Ready(_)) => {
                    trace!("Keep-alive timeout, close connection");
                    return Ok(Async::Ready(Http1Result::Done))
                }
                Ok(Async::NotReady) => (),
                Err(_) => unreachable!(),
            }
        }

        loop {
            let mut not_ready = true;

            // check in-flight messages
            let mut io = false;
            let mut idx = 0;
            while idx < self.tasks.len() {
                let item = &mut self.tasks[idx];

                if !io && !item.flags.contains(EntryFlags::EOF) {
                    if item.flags.contains(EntryFlags::ERROR) {
                        // check stream state
                        if let Ok(Async::NotReady) = self.stream.poll_completed() {
                            return Ok(Async::NotReady)
                        }
                        return Err(())
                    }

                    match item.pipe.poll_io(&mut self.stream) {
                        Ok(Async::Ready(ready)) => {
                            not_ready = false;

                            // overide keep-alive state
                            if self.settings.keep_alive_enabled() {
                                if self.stream.keepalive() {
                                    self.flags.insert(Flags::KEEPALIVE);
                                } else {
                                    self.flags.remove(Flags::KEEPALIVE);
                                }
                            }
                            self.stream.reset();

                            item.flags.insert(EntryFlags::EOF);
                            if ready {
                                item.flags.insert(EntryFlags::FINISHED);
                            }
                        },
                        Ok(Async::NotReady) => {
                            // no more IO for this iteration
                            io = true;
                        },
                        Err(err) => {
                            // it is not possible to recover from error
                            // during pipe handling, so just drop connection
                            error!("Unhandled error: {}", err);
                            item.flags.insert(EntryFlags::ERROR);

                            // check stream state, we still can have valid data in buffer
                            if let Ok(Async::NotReady) = self.stream.poll_completed() {
                                return Ok(Async::NotReady)
                            }
                            return Err(())
                        }
                    }
                } else if !item.flags.contains(EntryFlags::FINISHED) {
                    match item.pipe.poll() {
                        Ok(Async::NotReady) => (),
                        Ok(Async::Ready(_)) => {
                            not_ready = false;
                            item.flags.insert(EntryFlags::FINISHED);
                        },
                        Err(err) => {
                            item.flags.insert(EntryFlags::ERROR);
                            error!("Unhandled error: {}", err);
                        }
                    }
                }
                idx += 1;
            }

            // cleanup finished tasks
            while !self.tasks.is_empty() {
                if self.tasks[0].flags.contains(EntryFlags::EOF) &&
                    self.tasks[0].flags.contains(EntryFlags::FINISHED)
                {
                    self.tasks.pop_front();
                } else {
                    break
                }
            }

            // no keep-alive
            if !self.flags.contains(Flags::KEEPALIVE) && self.tasks.is_empty() {
                // check stream state
                if self.poll_completed()? {
                    return Ok(Async::NotReady)
                }
                if self.flags.contains(Flags::H2) {
                    return Ok(Async::Ready(Http1Result::Switch))
                } else {
                    return Ok(Async::Ready(Http1Result::Done))
                }
            }

            // read incoming data
            while !self.flags.contains(Flags::ERROR) && !self.flags.contains(Flags::H2) &&
                self.tasks.len() < MAX_PIPELINED_MESSAGES
            {
                match self.reader.parse(self.stream.get_mut(),
                                        &mut self.read_buf, &self.settings) {
                    Ok(Async::Ready(Item::Http1(mut req))) => {
                        not_ready = false;

                        // set remote addr
                        req.set_peer_addr(self.addr);

                        // stop keepalive timer
                        self.keepalive_timer.take();

                        // start request processing
                        let mut pipe = None;
                        for h in self.settings.handlers().iter() {
                            req = match h.handle(req) {
                                Ok(t) => {
                                    pipe = Some(t);
                                    break
                                },
                                Err(req) => req,
                            }
                        }

                        self.tasks.push_back(
                            Entry {pipe: pipe.unwrap_or_else(|| Pipeline::error(HTTPNotFound)),
                                   flags: EntryFlags::empty()});
                    }
                    Ok(Async::Ready(Item::Http2)) => {
                        self.flags.insert(Flags::H2);
                    }
                    Err(ReaderError::Disconnect) => {
                        not_ready = false;
                        self.flags.insert(Flags::ERROR);
                        self.stream.disconnected();
                        for entry in &mut self.tasks {
                            entry.pipe.disconnected()
                        }
                    },
                    Err(err) => {
                        // notify all tasks
                        not_ready = false;
                        self.stream.disconnected();
                        for entry in &mut self.tasks {
                            entry.pipe.disconnected()
                        }

                        // kill keepalive
                        self.flags.remove(Flags::KEEPALIVE);
                        self.keepalive_timer.take();

                        // on parse error, stop reading stream but tasks need to be completed
                        self.flags.insert(Flags::ERROR);

                        if self.tasks.is_empty() {
                            if let ReaderError::Error(err) = err {
                                self.tasks.push_back(
                                    Entry {pipe: Pipeline::error(err.error_response()),
                                           flags: EntryFlags::empty()});
                            }
                        }
                    }
                    Ok(Async::NotReady) => {
                        // start keep-alive timer, this is also slow request timeout
                        if self.tasks.is_empty() {
                            if self.settings.keep_alive_enabled() {
                                let keep_alive = self.settings.keep_alive();
                                if keep_alive > 0 && self.flags.contains(Flags::KEEPALIVE) {
                                    if self.keepalive_timer.is_none() {
                                        trace!("Start keep-alive timer");
                                        let mut to = Timeout::new(
                                            Duration::new(keep_alive, 0),
                                            Arbiter::handle()).unwrap();
                                        // register timeout
                                        let _ = to.poll();
                                        self.keepalive_timer = Some(to);
                                    }
                                } else {
                                    // check stream state
                                    if self.poll_completed()? {
                                        return Ok(Async::NotReady)
                                    }
                                    // keep-alive disable, drop connection
                                    return Ok(Async::Ready(Http1Result::Done))
                                }
                            } else {
                                // check stream state
                                self.poll_completed()?;
                                // keep-alive unset, rely on operating system
                                return Ok(Async::NotReady)
                            }
                        }
                        break
                    }
                }
            }

            // check for parse error
            if self.tasks.is_empty() {
                // check stream state
                if self.poll_completed()? {
                    return Ok(Async::NotReady)
                }

                if self.flags.contains(Flags::H2) {
                    return Ok(Async::Ready(Http1Result::Switch))
                }
                if self.flags.contains(Flags::ERROR) || self.keepalive_timer.is_none() {
                    return Ok(Async::Ready(Http1Result::Done))
                }
            }

            if not_ready {
                self.poll_completed()?;
                return Ok(Async::NotReady)
            }
        }
    }
}

struct Reader {
    h1: bool,
    payload: Option<PayloadInfo>,
}

enum Decoding {
    Paused,
    Ready,
    NotReady,
}

struct PayloadInfo {
    tx: PayloadType,
    decoder: Decoder,
}

#[derive(Debug)]
enum ReaderError {
    Disconnect,
    Payload,
    Error(ParseError),
}

enum Message {
    Http1(HttpRequest, Option<PayloadInfo>),
    Http2,
    NotReady,
}

impl Reader {
    pub fn new() -> Reader {
        Reader {
            h1: false,
            payload: None,
        }
    }

    fn decode(&mut self, buf: &mut BytesMut) -> std::result::Result<Decoding, ReaderError>
    {
        if let Some(ref mut payload) = self.payload {
            if payload.tx.capacity() > DEFAULT_BUFFER_SIZE {
                return Ok(Decoding::Paused)
            }
            loop {
                match payload.decoder.decode(buf) {
                    Ok(Async::Ready(Some(bytes))) => {
                        payload.tx.feed_data(bytes)
                    },
                    Ok(Async::Ready(None)) => {
                        payload.tx.feed_eof();
                        return Ok(Decoding::Ready)
                    },
                    Ok(Async::NotReady) => return Ok(Decoding::NotReady),
                    Err(err) => {
                        payload.tx.set_error(err.into());
                        return Err(ReaderError::Payload)
                    }
                }
            }
        } else {
            return Ok(Decoding::Ready)
        }
    }
    
    pub fn parse<T, H>(&mut self, io: &mut T,
                       buf: &mut BytesMut,
                       settings: &WorkerSettings<H>) -> Poll<Item, ReaderError>
        where T: AsyncRead
    {
        // read payload
        if self.payload.is_some() {
            match self.read_from_io(io, buf) {
                Ok(Async::Ready(0)) => {
                    if let Some(ref mut payload) = self.payload {
                        payload.tx.set_error(PayloadError::Incomplete);
                    }
                    // http channel should not deal with payload errors
                    return Err(ReaderError::Payload)
                },
                Err(err) => {
                    if let Some(ref mut payload) = self.payload {
                        payload.tx.set_error(err.into());
                    }
                    // http channel should not deal with payload errors
                    return Err(ReaderError::Payload)
                }
                _ => (),
            }
            match self.decode(buf)? {
                Decoding::Ready => self.payload = None,
                Decoding::Paused | Decoding::NotReady => return Ok(Async::NotReady),
            }
        }

        // if buf is empty parse_message will always return NotReady, let's avoid that
        let read = if buf.is_empty() {
            match self.read_from_io(io, buf) {
                Ok(Async::Ready(0)) => {
                    // debug!("Ignored premature client disconnection");
                    return Err(ReaderError::Disconnect);
                },
                Ok(Async::Ready(_)) => (),
                Ok(Async::NotReady) =>
                    return Ok(Async::NotReady),
                Err(err) =>
                    return Err(ReaderError::Error(err.into()))
            }
            false
        } else {
            true
        };

        loop {
            match Reader::parse_message(buf, settings).map_err(ReaderError::Error)? {
                Message::Http1(msg, decoder) => {
                    // process payload
                    if let Some(payload) = decoder {
                        self.payload = Some(payload);
                        match self.decode(buf)? {
                            Decoding::Paused | Decoding::NotReady => (),
                            Decoding::Ready => self.payload = None,
                        }
                    }
                    self.h1 = true;
                    return Ok(Async::Ready(Item::Http1(msg)));
                },
                Message::Http2 => {
                    if self.h1 {
                        return Err(ReaderError::Error(ParseError::Version))
                    }
                    return Ok(Async::Ready(Item::Http2));
                },
                Message::NotReady => {
                    if buf.capacity() >= MAX_BUFFER_SIZE {
                        error!("MAX_BUFFER_SIZE unprocessed data reached, closing");
                        return Err(ReaderError::Error(ParseError::TooLarge));
                    }
                    if read {
                        match self.read_from_io(io, buf) {
                            Ok(Async::Ready(0)) => {
                                debug!("Ignored premature client disconnection");
                                return Err(ReaderError::Disconnect);
                            },
                            Ok(Async::Ready(_)) => (),
                            Ok(Async::NotReady) =>
                                return Ok(Async::NotReady),
                            Err(err) =>
                                return Err(ReaderError::Error(err.into()))
                        }
                    } else {
                        return Ok(Async::NotReady)
                    }
                },
            }
        }
    }

    fn read_from_io<T: AsyncRead>(&mut self, io: &mut T, buf: &mut BytesMut)
                                  -> Poll<usize, io::Error>
    {
        unsafe {
            if buf.remaining_mut() < LW_BUFFER_SIZE {
                buf.reserve(HW_BUFFER_SIZE);
            }
            match io.read(buf.bytes_mut()) {
                Ok(n) => {
                    buf.advance_mut(n);
                    Ok(Async::Ready(n))
                },
                Err(e) => {
                    if e.kind() == io::ErrorKind::WouldBlock {
                        Ok(Async::NotReady)
                    } else {
                        Err(e)
                    }
                }
            }
        }
    }

    fn parse_message<H>(buf: &mut BytesMut, settings: &WorkerSettings<H>)
                        -> Result<Message, ParseError>
    {
        if buf.is_empty() {
            return Ok(Message::NotReady);
        }
        if buf.len() >= 14 && buf[..14] == HTTP2_PREFACE[..] {
            return Ok(Message::Http2)
        }

        // Parse http message
        let msg = {
            let bytes_ptr = buf.as_ref().as_ptr() as usize;
            let mut headers: [httparse::Header; MAX_HEADERS] =
                unsafe{std::mem::uninitialized()};

            let (len, method, path, version, headers_len) = {
                let b = unsafe{ let b: &[u8] = buf; std::mem::transmute(b) };
                let mut req = httparse::Request::new(&mut headers);
                match req.parse(b)? {
                    httparse::Status::Complete(len) => {
                        let method = Method::try_from(req.method.unwrap())
                            .map_err(|_| ParseError::Method)?;
                        let path = req.path.unwrap();
                        let path_start = path.as_ptr() as usize - bytes_ptr;
                        let path_end = path_start + path.len();
                        let path = (path_start, path_end);

                        let version = if req.version.unwrap() == 1 {
                            Version::HTTP_11
                        } else {
                            Version::HTTP_10
                        };
                        (len, method, path, version, req.headers.len())
                    }
                    httparse::Status::Partial => return Ok(Message::NotReady),
                }
            };

            let slice = buf.split_to(len).freeze();

            // convert headers
            let msg = settings.get_http_message();
            for header in headers[..headers_len].iter() {
                if let Ok(name) = HeaderName::try_from(header.name) {
                    let v_start = header.value.as_ptr() as usize - bytes_ptr;
                    let v_end = v_start + header.value.len();
                    let value = unsafe {
                        HeaderValue::from_shared_unchecked(slice.slice(v_start, v_end)) };
                    msg.get_mut().headers.append(name, value);
                } else {
                    return Err(ParseError::Header)
                }
            }

            let path = slice.slice(path.0, path.1);
            let uri = Uri::from_shared(path).map_err(ParseError::Uri)?;

            msg.get_mut().uri = uri;
            msg.get_mut().method = method;
            msg.get_mut().version = version;
            msg
        };

        let decoder = if let Some(len) = msg.get_ref().headers.get(header::CONTENT_LENGTH) {
            // Content-Length
            if let Ok(s) = len.to_str() {
                if let Ok(len) = s.parse::<u64>() {
                    Some(Decoder::length(len))
                } else {
                    debug!("illegal Content-Length: {:?}", len);
                    return Err(ParseError::Header)
                }
            } else {
                debug!("illegal Content-Length: {:?}", len);
                return Err(ParseError::Header)
            }
        } else if chunked(&msg.get_mut().headers)? {
            // Chunked encoding
            Some(Decoder::chunked())
        } else if msg.get_ref().headers.contains_key(header::UPGRADE) ||
            msg.get_ref().method == Method::CONNECT
        {
            Some(Decoder::eof())
        } else {
            None
        };

        if let Some(decoder) = decoder {
            let (psender, payload) = Payload::new(false);
            let info = PayloadInfo {
                tx: PayloadType::new(&msg.get_mut().headers, psender),
                decoder: decoder,
            };
            msg.get_mut().payload = Some(payload);
            Ok(Message::Http1(HttpRequest::from_message(msg), Some(info)))
        } else {
            Ok(Message::Http1(HttpRequest::from_message(msg), None))
        }
    }
}

/// Check if request has chunked transfer encoding
fn chunked(headers: &HeaderMap) -> Result<bool, ParseError> {
    if let Some(encodings) = headers.get(header::TRANSFER_ENCODING) {
        if let Ok(s) = encodings.to_str() {
            Ok(s.to_lowercase().contains("chunked"))
        } else {
            Err(ParseError::Header)
        }
    } else {
        Ok(false)
    }
}

/// Decoders to handle different Transfer-Encodings.
///
/// If a message body does not include a Transfer-Encoding, it *should*
/// include a Content-Length header.
#[derive(Debug, Clone, PartialEq)]
struct Decoder {
    kind: Kind,
}

impl Decoder {
    pub fn length(x: u64) -> Decoder {
        Decoder { kind: Kind::Length(x) }
    }

    pub fn chunked() -> Decoder {
        Decoder { kind: Kind::Chunked(ChunkedState::Size, 0) }
    }

    pub fn eof() -> Decoder {
        Decoder { kind: Kind::Eof(false) }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    /// A Reader used when a Content-Length header is passed with a positive integer.
    Length(u64),
    /// A Reader used when Transfer-Encoding is `chunked`.
    Chunked(ChunkedState, u64),
    /// A Reader used for responses that don't indicate a length or chunked.
    ///
    /// Note: This should only used for `Response`s. It is illegal for a
    /// `Request` to be made with both `Content-Length` and
    /// `Transfer-Encoding: chunked` missing, as explained from the spec:
    ///
    /// > If a Transfer-Encoding header field is present in a response and
    /// > the chunked transfer coding is not the final encoding, the
    /// > message body length is determined by reading the connection until
    /// > it is closed by the server.  If a Transfer-Encoding header field
    /// > is present in a request and the chunked transfer coding is not
    /// > the final encoding, the message body length cannot be determined
    /// > reliably; the server MUST respond with the 400 (Bad Request)
    /// > status code and then close the connection.
    Eof(bool),
}

#[derive(Debug, PartialEq, Clone)]
enum ChunkedState {
    Size,
    SizeLws,
    Extension,
    SizeLf,
    Body,
    BodyCr,
    BodyLf,
    EndCr,
    EndLf,
    End,
}

impl Decoder {
    pub fn decode(&mut self, body: &mut BytesMut) -> Poll<Option<Bytes>, io::Error> {
        match self.kind {
            Kind::Length(ref mut remaining) => {
                if *remaining == 0 {
                    Ok(Async::Ready(None))
                } else {
                    if body.is_empty() {
                        return Ok(Async::NotReady)
                    }
                    let len = body.len() as u64;
                    let buf;
                    if *remaining > len {
                        buf = body.take().freeze();
                        *remaining -= len;
                    } else {
                        buf = body.split_to(*remaining as usize).freeze();
                        *remaining = 0;
                    }
                    trace!("Length read: {}", buf.len());
                    Ok(Async::Ready(Some(buf)))
                }
            }
            Kind::Chunked(ref mut state, ref mut size) => {
                loop {
                    let mut buf = None;
                    // advances the chunked state
                    *state = try_ready!(state.step(body, size, &mut buf));
                    if *state == ChunkedState::End {
                        trace!("End of chunked stream");
                        return Ok(Async::Ready(None));
                    }
                    if let Some(buf) = buf {
                        return Ok(Async::Ready(Some(buf)));
                    }
                    if body.is_empty() {
                        return Ok(Async::NotReady);
                    }
                }
            }
            Kind::Eof(ref mut is_eof) => {
                if *is_eof {
                    Ok(Async::Ready(None))
                } else if !body.is_empty() {
                    Ok(Async::Ready(Some(body.take().freeze())))
                } else {
                    Ok(Async::NotReady)
                }
            }
        }
    }
}

macro_rules! byte (
    ($rdr:ident) => ({
        if $rdr.len() > 0 {
            let b = $rdr[0];
            $rdr.split_to(1);
            b
        } else {
            return Ok(Async::NotReady)
        }
    })
);

impl ChunkedState {
    fn step(&self, body: &mut BytesMut, size: &mut u64, buf: &mut Option<Bytes>)
            -> Poll<ChunkedState, io::Error>
    {
        use self::ChunkedState::*;
        match *self {
            Size => ChunkedState::read_size(body, size),
            SizeLws => ChunkedState::read_size_lws(body),
            Extension => ChunkedState::read_extension(body),
            SizeLf => ChunkedState::read_size_lf(body, size),
            Body => ChunkedState::read_body(body, size, buf),
            BodyCr => ChunkedState::read_body_cr(body),
            BodyLf => ChunkedState::read_body_lf(body),
            EndCr => ChunkedState::read_end_cr(body),
            EndLf => ChunkedState::read_end_lf(body),
            End => Ok(Async::Ready(ChunkedState::End)),
        }
    }
    fn read_size(rdr: &mut BytesMut, size: &mut u64) -> Poll<ChunkedState, io::Error> {
        let radix = 16;
        match byte!(rdr) {
            b @ b'0'...b'9' => {
                *size *= radix;
                *size += u64::from(b - b'0');
            }
            b @ b'a'...b'f' => {
                *size *= radix;
                *size += u64::from(b + 10 - b'a');
            }
            b @ b'A'...b'F' => {
                *size *= radix;
                *size += u64::from(b + 10 - b'A');
            }
            b'\t' | b' ' => return Ok(Async::Ready(ChunkedState::SizeLws)),
            b';' => return Ok(Async::Ready(ChunkedState::Extension)),
            b'\r' => return Ok(Async::Ready(ChunkedState::SizeLf)),
            _ => {
                return Err(io::Error::new(io::ErrorKind::InvalidInput,
                                          "Invalid chunk size line: Invalid Size"));
            }
        }
        Ok(Async::Ready(ChunkedState::Size))
    }
    fn read_size_lws(rdr: &mut BytesMut) -> Poll<ChunkedState, io::Error> {
        trace!("read_size_lws");
        match byte!(rdr) {
            // LWS can follow the chunk size, but no more digits can come
            b'\t' | b' ' => Ok(Async::Ready(ChunkedState::SizeLws)),
            b';' => Ok(Async::Ready(ChunkedState::Extension)),
            b'\r' => Ok(Async::Ready(ChunkedState::SizeLf)),
            _ => {
                Err(io::Error::new(io::ErrorKind::InvalidInput,
                                   "Invalid chunk size linear white space"))
            }
        }
    }
    fn read_extension(rdr: &mut BytesMut) -> Poll<ChunkedState, io::Error> {
        match byte!(rdr) {
            b'\r' => Ok(Async::Ready(ChunkedState::SizeLf)),
            _ => Ok(Async::Ready(ChunkedState::Extension)), // no supported extensions
        }
    }
    fn read_size_lf(rdr: &mut BytesMut, size: &mut u64) -> Poll<ChunkedState, io::Error> {
        match byte!(rdr) {
            b'\n' if *size > 0 => Ok(Async::Ready(ChunkedState::Body)),
            b'\n' if *size == 0 => Ok(Async::Ready(ChunkedState::EndCr)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk size LF")),
        }
    }

    fn read_body(rdr: &mut BytesMut, rem: &mut u64, buf: &mut Option<Bytes>)
                 -> Poll<ChunkedState, io::Error>
    {
        trace!("Chunked read, remaining={:?}", rem);

        let len = rdr.len() as u64;
        if len == 0 {
            Ok(Async::Ready(ChunkedState::Body))
        } else {
            let slice;
            if *rem > len {
                slice = rdr.take().freeze();
                *rem -= len;
            } else {
                slice = rdr.split_to(*rem as usize).freeze();
                *rem = 0;
            }
            *buf = Some(slice);
            if *rem > 0 {
                Ok(Async::Ready(ChunkedState::Body))
            } else {
                Ok(Async::Ready(ChunkedState::BodyCr))
            }
        }
    }

    fn read_body_cr(rdr: &mut BytesMut) -> Poll<ChunkedState, io::Error> {
        match byte!(rdr) {
            b'\r' => Ok(Async::Ready(ChunkedState::BodyLf)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk body CR")),
        }
    }
    fn read_body_lf(rdr: &mut BytesMut) -> Poll<ChunkedState, io::Error> {
        match byte!(rdr) {
            b'\n' => Ok(Async::Ready(ChunkedState::Size)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk body LF")),
        }
    }
    fn read_end_cr(rdr: &mut BytesMut) -> Poll<ChunkedState, io::Error> {
        match byte!(rdr) {
            b'\r' => Ok(Async::Ready(ChunkedState::EndLf)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk end CR")),
        }
    }
    fn read_end_lf(rdr: &mut BytesMut) -> Poll<ChunkedState, io::Error> {
        match byte!(rdr) {
            b'\n' => Ok(Async::Ready(ChunkedState::End)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk end LF")),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{io, cmp};
    use bytes::{Bytes, BytesMut};
    use futures::{Async};
    use tokio_io::AsyncRead;
    use http::{Version, Method};
    use super::*;
    use application::HttpApplication;
    use server::WorkerSettings;

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
        fn feed_data(&mut self, data: &'static str) {
            let mut b = BytesMut::from(self.buf.as_ref());
            b.extend(data.as_bytes());
            self.buf = b.take().freeze();
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

    macro_rules! not_ready {
        ($e:expr) => (match $e {
            Ok(Async::NotReady) => (),
            Err(err) => panic!("Unexpected error: {:?}", err),
            _ => panic!("Should not be ready"),
        })
    }

    macro_rules! parse_ready {
        ($e:expr) => ({
            let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), None);
            match Reader::new().parse($e, &mut BytesMut::new(), &settings) {
                Ok(Async::Ready(Item::Http1(req))) => req,
                Ok(_) => panic!("Eof during parsing http request"),
                Err(err) => panic!("Error during parsing http request: {:?}", err),
            }
        })
    }

    macro_rules! reader_parse_ready {
        ($e:expr) => (
            match $e {
                Ok(Async::Ready(Item::Http1(req))) => req,
                Ok(_) => panic!("Eof during parsing http request"),
                Err(err) => panic!("Error during parsing http request: {:?}", err),
            }
        )
    }

    macro_rules! expect_parse_err {
        ($e:expr) => ({
            let mut buf = BytesMut::new();
            let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), None);

            match Reader::new().parse($e, &mut buf, &settings) {
                Err(err) => match err {
                    ReaderError::Error(_) => (),
                    _ => panic!("Parse error expected"),
                },
                _ => {
                    panic!("Error expected")
                }
            }}
        )
    }

    #[test]
    fn test_parse() {
        let mut buf = Buffer::new("GET /test HTTP/1.1\r\n\r\n");
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), None);

        let mut reader = Reader::new();
        match reader.parse(&mut buf, &mut readbuf, &settings) {
            Ok(Async::Ready(Item::Http1(req))) => {
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
            }
            Ok(_) | Err(_) => panic!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_partial() {
        let mut buf = Buffer::new("PUT /test HTTP/1");
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), None);

        let mut reader = Reader::new();
        match reader.parse(&mut buf, &mut readbuf, &settings) {
            Ok(Async::NotReady) => (),
            _ => panic!("Error"),
        }

        buf.feed_data(".1\r\n\r\n");
        match reader.parse(&mut buf, &mut readbuf, &settings) {
            Ok(Async::Ready(Item::Http1(req))) => {
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::PUT);
                assert_eq!(req.path(), "/test");
            }
            Ok(_) | Err(_) => panic!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_post() {
        let mut buf = Buffer::new("POST /test2 HTTP/1.0\r\n\r\n");
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), None);

        let mut reader = Reader::new();
        match reader.parse(&mut buf, &mut readbuf, &settings) {
            Ok(Async::Ready(Item::Http1(req))) => {
                assert_eq!(req.version(), Version::HTTP_10);
                assert_eq!(*req.method(), Method::POST);
                assert_eq!(req.path(), "/test2");
            }
            Ok(_) | Err(_) => panic!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_body() {
        let mut buf = Buffer::new("GET /test HTTP/1.1\r\nContent-Length: 4\r\n\r\nbody");
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), None);

        let mut reader = Reader::new();
        match reader.parse(&mut buf, &mut readbuf, &settings) {
            Ok(Async::Ready(Item::Http1(mut req))) => {
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
                assert_eq!(req.payload_mut().readall().unwrap().as_ref(), b"body");
            }
            Ok(_) | Err(_) => panic!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_body_crlf() {
        let mut buf = Buffer::new(
            "\r\nGET /test HTTP/1.1\r\nContent-Length: 4\r\n\r\nbody");
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), None);

        let mut reader = Reader::new();
        match reader.parse(&mut buf, &mut readbuf, &settings) {
            Ok(Async::Ready(Item::Http1(mut req))) => {
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
                assert_eq!(req.payload_mut().readall().unwrap().as_ref(), b"body");
            }
            Ok(_) | Err(_) => panic!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_partial_eof() {
        let mut buf = Buffer::new("GET /test HTTP/1.1\r\n");
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), None);

        let mut reader = Reader::new();
        not_ready!{ reader.parse(&mut buf, &mut readbuf, &settings) }

        buf.feed_data("\r\n");
        match reader.parse(&mut buf, &mut readbuf, &settings) {
            Ok(Async::Ready(Item::Http1(req))) => {
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
            }
            Ok(_) | Err(_) => panic!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_headers_split_field() {
        let mut buf = Buffer::new("GET /test HTTP/1.1\r\n");
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), None);

        let mut reader = Reader::new();
        not_ready!{ reader.parse(&mut buf, &mut readbuf, &settings) }

        buf.feed_data("t");
        not_ready!{ reader.parse(&mut buf, &mut readbuf, &settings) }

        buf.feed_data("es");
        not_ready!{ reader.parse(&mut buf, &mut readbuf, &settings) }

        buf.feed_data("t: value\r\n\r\n");
        match reader.parse(&mut buf, &mut readbuf, &settings) {
            Ok(Async::Ready(Item::Http1(req))) => {
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
                assert_eq!(req.headers().get("test").unwrap().as_bytes(), b"value");
            }
            Ok(_) | Err(_) => panic!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_headers_multi_value() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             Set-Cookie: c1=cookie1\r\n\
             Set-Cookie: c2=cookie2\r\n\r\n");
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), None);

        let mut reader = Reader::new();
        match reader.parse(&mut buf, &mut readbuf, &settings) {
            Ok(Async::Ready(Item::Http1(req))) => {
                let val: Vec<_> = req.headers().get_all("Set-Cookie")
                    .iter().map(|v| v.to_str().unwrap().to_owned()).collect();
                assert_eq!(val[0], "c1=cookie1");
                assert_eq!(val[1], "c2=cookie2");
            }
            Ok(_) | Err(_) => panic!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_conn_default_1_0() {
        let mut buf = Buffer::new("GET /test HTTP/1.0\r\n\r\n");
        let req = parse_ready!(&mut buf);

        assert!(!req.keep_alive());
    }

    #[test]
    fn test_conn_default_1_1() {
        let mut buf = Buffer::new("GET /test HTTP/1.1\r\n\r\n");
        let req = parse_ready!(&mut buf);

        assert!(req.keep_alive());
    }

    #[test]
    fn test_conn_close() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             connection: close\r\n\r\n");
        let req = parse_ready!(&mut buf);

        assert!(!req.keep_alive());
    }

    #[test]
    fn test_conn_close_1_0() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.0\r\n\
             connection: close\r\n\r\n");
        let req = parse_ready!(&mut buf);

        assert!(!req.keep_alive());
    }

    #[test]
    fn test_conn_keep_alive_1_0() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.0\r\n\
             connection: keep-alive\r\n\r\n");
        let req = parse_ready!(&mut buf);

        assert!(req.keep_alive());
    }

    #[test]
    fn test_conn_keep_alive_1_1() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             connection: keep-alive\r\n\r\n");
        let req = parse_ready!(&mut buf);

        assert!(req.keep_alive());
    }

    #[test]
    fn test_conn_other_1_0() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.0\r\n\
             connection: other\r\n\r\n");
        let req = parse_ready!(&mut buf);

        assert!(!req.keep_alive());
    }

    #[test]
    fn test_conn_other_1_1() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             connection: other\r\n\r\n");
        let req = parse_ready!(&mut buf);

        assert!(req.keep_alive());
    }

    #[test]
    fn test_conn_upgrade() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             connection: upgrade\r\n\r\n");
        let req = parse_ready!(&mut buf);

        assert!(!req.payload().eof());
        assert!(req.upgrade());
    }

    #[test]
    fn test_conn_upgrade_connect_method() {
        let mut buf = Buffer::new(
            "CONNECT /test HTTP/1.1\r\n\
             content-length: 0\r\n\r\n");
        let req = parse_ready!(&mut buf);

        assert!(req.upgrade());
        assert!(!req.payload().eof());
    }

    #[test]
    fn test_request_chunked() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n");
        let req = parse_ready!(&mut buf);

        if let Ok(val) = req.chunked() {
            assert!(val);
        } else {
            panic!("Error");
        }

        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chnked\r\n\r\n");
        let req = parse_ready!(&mut buf);

        if let Ok(val) = req.chunked() {
            assert!(!val);
        } else {
            panic!("Error");
        }
    }

    #[test]
    fn test_headers_content_length_err_1() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             content-length: line\r\n\r\n");

        expect_parse_err!(&mut buf)
    }

    #[test]
    fn test_headers_content_length_err_2() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             content-length: -1\r\n\r\n");

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_invalid_header() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             test line\r\n\r\n");

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_invalid_name() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             test[]: line\r\n\r\n");

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_http_request_bad_status_line() {
        let mut buf = Buffer::new("getpath \r\n\r\n");
        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_http_request_upgrade() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             connection: upgrade\r\n\
             upgrade: websocket\r\n\r\n\
             some raw data");
        let mut req = parse_ready!(&mut buf);
        assert!(!req.keep_alive());
        assert!(req.upgrade());
        assert_eq!(req.payload_mut().readall().unwrap().as_ref(), b"some raw data");
    }

    #[test]
    fn test_http_request_parser_utf8() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             x-test: тест\r\n\r\n");
        let req = parse_ready!(&mut buf);

        assert_eq!(req.headers().get("x-test").unwrap().as_bytes(),
                   "тест".as_bytes());
    }

    #[test]
    fn test_http_request_parser_two_slashes() {
        let mut buf = Buffer::new(
            "GET //path HTTP/1.1\r\n\r\n");
        let req = parse_ready!(&mut buf);

        assert_eq!(req.path(), "//path");
    }

    #[test]
    fn test_http_request_parser_bad_method() {
        let mut buf = Buffer::new(
            "!12%()+=~$ /get HTTP/1.1\r\n\r\n");

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_http_request_parser_bad_version() {
        let mut buf = Buffer::new("GET //get HT/11\r\n\r\n");

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_http_request_chunked_payload() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n");
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), None);

        let mut reader = Reader::new();
        let mut req = reader_parse_ready!(reader.parse(&mut buf, &mut readbuf, &settings));
        assert!(req.chunked().unwrap());
        assert!(!req.payload().eof());

        buf.feed_data("4\r\ndata\r\n4\r\nline\r\n0\r\n\r\n");
        not_ready!(reader.parse(&mut buf, &mut readbuf, &settings));
        assert!(!req.payload().eof());
        assert_eq!(req.payload_mut().readall().unwrap().as_ref(), b"dataline");
        assert!(req.payload().eof());
    }

    #[test]
    fn test_http_request_chunked_payload_and_next_message() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n");
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), None);

        let mut reader = Reader::new();

        let mut req = reader_parse_ready!(reader.parse(&mut buf, &mut readbuf, &settings));
        assert!(req.chunked().unwrap());
        assert!(!req.payload().eof());

        buf.feed_data(
            "4\r\ndata\r\n4\r\nline\r\n0\r\n\r\n\
             POST /test2 HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n");

        let req2 = reader_parse_ready!(reader.parse(&mut buf, &mut readbuf, &settings));
        assert_eq!(*req2.method(), Method::POST);
        assert!(req2.chunked().unwrap());
        assert!(!req2.payload().eof());

        assert_eq!(req.payload_mut().readall().unwrap().as_ref(), b"dataline");
        assert!(req.payload().eof());
    }

    #[test]
    fn test_http_request_chunked_payload_chunks() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n");
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), None);

        let mut reader = Reader::new();
        let mut req = reader_parse_ready!(reader.parse(&mut buf, &mut readbuf, &settings));
        assert!(req.chunked().unwrap());
        assert!(!req.payload().eof());

        buf.feed_data("4\r\ndata\r");
        not_ready!(reader.parse(&mut buf, &mut readbuf, &settings));

        buf.feed_data("\n4");
        not_ready!(reader.parse(&mut buf, &mut readbuf, &settings));

        buf.feed_data("\r");
        not_ready!(reader.parse(&mut buf, &mut readbuf, &settings));
        buf.feed_data("\n");
        not_ready!(reader.parse(&mut buf, &mut readbuf, &settings));

        buf.feed_data("li");
        not_ready!(reader.parse(&mut buf, &mut readbuf, &settings));

        buf.feed_data("ne\r\n0\r\n");
        not_ready!(reader.parse(&mut buf, &mut readbuf, &settings));

        //buf.feed_data("test: test\r\n");
        //not_ready!(reader.parse(&mut buf, &mut readbuf));

        assert_eq!(req.payload_mut().readall().unwrap().as_ref(), b"dataline");
        assert!(!req.payload().eof());

        buf.feed_data("\r\n");
        not_ready!(reader.parse(&mut buf, &mut readbuf, &settings));
        assert!(req.payload().eof());
    }

    #[test]
    fn test_parse_chunked_payload_chunk_extension() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n");
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), None);

        let mut reader = Reader::new();
        let mut req = reader_parse_ready!(reader.parse(&mut buf, &mut readbuf, &settings));
        assert!(req.chunked().unwrap());
        assert!(!req.payload().eof());

        buf.feed_data("4;test\r\ndata\r\n4\r\nline\r\n0\r\n\r\n"); // test: test\r\n\r\n")
        not_ready!(reader.parse(&mut buf, &mut readbuf, &settings));
        assert!(!req.payload().eof());
        assert_eq!(req.payload_mut().readall().unwrap().as_ref(), b"dataline");
        assert!(req.payload().eof());
    }

    /*#[test]
    #[should_panic]
    fn test_parse_multiline() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             test: line\r\n \
               continue\r\n\
             test2: data\r\n\
             \r\n", false);

        let mut reader = Reader::new();
        match reader.parse(&mut buf) {
            Ok(res) => (),
            Err(err) => panic!("{:?}", err),
        }
    }*/

    #[test]
    fn test_http2_prefix() {
        let mut buf = Buffer::new("PRI * HTTP/2.0\r\n\r\n");
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), None);

        let mut reader = Reader::new();
        match reader.parse(&mut buf, &mut readbuf, &settings) {
            Ok(Async::Ready(Item::Http2)) => (),
            Ok(_) | Err(_) => panic!("Error during parsing http request"),
        }
    }
}
