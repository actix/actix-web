use std::{io, fmt, cmp};
use std::rc::{Rc, Weak};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::error::Error;
use std::io::{Read, Write, Error as IoError};
use bytes::{Bytes, BytesMut, BufMut, Writer};
use http2::Error as Http2Error;
use futures::{Async, Poll, Stream};
use futures::task::{Task, current as current_task};
use flate2::read::{GzDecoder};
use flate2::write::{DeflateDecoder};
use brotli2::write::BrotliDecoder;

use actix::ResponseType;
use httpresponse::ContentEncoding;

pub(crate) const DEFAULT_BUFFER_SIZE: usize = 65_536; // max buffer size 64k

/// Just Bytes object
pub struct PayloadItem(pub Bytes);

impl ResponseType for PayloadItem {
    type Item = ();
    type Error = ();
}

impl fmt::Debug for PayloadItem {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

#[derive(Debug)]
/// A set of error that can occur during payload parsing.
pub enum PayloadError {
    /// A payload reached EOF, but is not complete.
    Incomplete,
    /// Content encoding stream corruption
    EncodingCorrupted,
    /// Parse error
    ParseError(IoError),
    /// Http2 error
    Http2(Http2Error),
}

impl fmt::Display for PayloadError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            PayloadError::ParseError(ref e) => fmt::Display::fmt(e, f),
            ref e => f.write_str(e.description()),
        }
    }
}

impl Error for PayloadError {
    fn description(&self) -> &str {
        match *self {
            PayloadError::Incomplete => "A payload reached EOF, but is not complete.",
            PayloadError::EncodingCorrupted => "Can not decode content-encoding.",
            PayloadError::ParseError(ref e) => e.description(),
            PayloadError::Http2(ref e) => e.description(),
        }
    }

    fn cause(&self) -> Option<&Error> {
        match *self {
            PayloadError::ParseError(ref error) => Some(error),
            _ => None,
        }
    }
}

impl From<IoError> for PayloadError {
    fn from(err: IoError) -> PayloadError {
        PayloadError::ParseError(err)
    }
}

/// Stream of byte chunks
///
/// Payload stores chunks in vector. First chunk can be received with `.readany()` method.
#[derive(Debug)]
pub struct Payload {
    inner: Rc<RefCell<Inner>>,
}

impl Payload {

    pub(crate) fn new(eof: bool) -> (PayloadSender, Payload) {
        let shared = Rc::new(RefCell::new(Inner::new(eof)));

        (PayloadSender{inner: Rc::downgrade(&shared)}, Payload{inner: shared})
    }

    /// Indicates EOF of payload
    pub fn eof(&self) -> bool {
        self.inner.borrow().eof()
    }

    /// Length of the data in this payload
    pub fn len(&self) -> usize {
        self.inner.borrow().len()
    }

    /// Is payload empty
    pub fn is_empty(&self) -> bool {
        self.inner.borrow().len() == 0
    }

    /// Get first available chunk of data.
    /// Returns Some(PayloadItem) as chunk, `None` indicates eof.
    pub fn readany(&mut self) -> Poll<Option<PayloadItem>, PayloadError> {
        self.inner.borrow_mut().readany()
    }

    /// Get exactly number of bytes
    /// Returns Some(PayloadItem) as chunk, `None` indicates eof.
    pub fn readexactly(&mut self, size: usize) -> Result<Async<Bytes>, PayloadError> {
        self.inner.borrow_mut().readexactly(size)
    }

    /// Read until `\n`
    /// Returns Some(PayloadItem) as line, `None` indicates eof.
    pub fn readline(&mut self) -> Result<Async<Bytes>, PayloadError> {
        self.inner.borrow_mut().readline()
    }

    /// Read until match line
    /// Returns Some(PayloadItem) as line, `None` indicates eof.
    pub fn readuntil(&mut self, line: &[u8]) -> Result<Async<Bytes>, PayloadError> {
        self.inner.borrow_mut().readuntil(line)
    }

    #[doc(hidden)]
    pub fn readall(&mut self) -> Option<Bytes> {
        self.inner.borrow_mut().readall()
    }

    /// Put unused data back to payload
    pub fn unread_data(&mut self, data: Bytes) {
        self.inner.borrow_mut().unread_data(data);
    }

    /// Get size of payload buffer
    pub fn buffer_size(&self) -> usize {
        self.inner.borrow().buffer_size()
    }

    /// Set size of payload buffer
    pub fn set_buffer_size(&self, size: usize) {
        self.inner.borrow_mut().set_buffer_size(size)
    }
}

impl Stream for Payload {
    type Item = PayloadItem;
    type Error = PayloadError;

    fn poll(&mut self) -> Poll<Option<PayloadItem>, PayloadError> {
        self.readany()
    }
}

pub(crate) trait PayloadWriter {
    fn set_error(&mut self, err: PayloadError);

    fn feed_eof(&mut self);

    fn feed_data(&mut self, data: Bytes);

    fn capacity(&self) -> usize;
}

pub(crate) struct PayloadSender {
    inner: Weak<RefCell<Inner>>,
}

impl PayloadWriter for PayloadSender {

    fn set_error(&mut self, err: PayloadError) {
        if let Some(shared) = self.inner.upgrade() {
            shared.borrow_mut().set_error(err)
        }
    }

    fn feed_eof(&mut self) {
        if let Some(shared) = self.inner.upgrade() {
            shared.borrow_mut().feed_eof()
        }
    }

    fn feed_data(&mut self, data: Bytes) {
        if let Some(shared) = self.inner.upgrade() {
            shared.borrow_mut().feed_data(data)
        }
    }

    fn capacity(&self) -> usize {
        if let Some(shared) = self.inner.upgrade() {
            shared.borrow().capacity()
        } else {
            0
        }
    }
}

enum Decoder {
    Zlib(DeflateDecoder<BytesWriter>),
    Gzip(Option<GzDecoder<Wrapper>>),
    Br(Rc<RefCell<BytesMut>>, BrotliDecoder<WrapperRc>),
    Identity,
}

// should go after write::GzDecoder get implemented
#[derive(Debug)]
struct Wrapper {
    buf: BytesMut
}

impl io::Read for Wrapper {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = cmp::min(buf.len(), self.buf.len());
        buf[..len].copy_from_slice(&self.buf[..len]);
        self.buf.split_to(len);
        Ok(len)
    }
}

struct BytesWriter {
    buf: BytesMut,
}

impl Default for BytesWriter {
    fn default() -> BytesWriter {
        BytesWriter{buf: BytesMut::with_capacity(8192)}
    }
}

impl io::Write for BytesWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.extend(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}


// should go after brotli2::write::BrotliDecoder::get_mut get implemented
#[derive(Debug)]
struct WrapperRc {
    buf: Rc<RefCell<BytesMut>>,
}

impl io::Write for WrapperRc {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.borrow_mut().extend(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) struct EncodedPayload {
    inner: PayloadSender,
    decoder: Decoder,
    dst: Writer<BytesMut>,
    error: bool,
}

impl EncodedPayload {
    pub fn new(inner: PayloadSender, enc: ContentEncoding) -> EncodedPayload {
        let dec = match enc {
            ContentEncoding::Deflate => Decoder::Zlib(
                DeflateDecoder::new(BytesWriter::default())),
            ContentEncoding::Gzip => Decoder::Gzip(None),
            ContentEncoding::Br => {
                let buf = Rc::new(RefCell::new(BytesMut::new()));
                let buf2 = Rc::clone(&buf);
                Decoder::Br(buf, BrotliDecoder::new(WrapperRc{buf: buf2}))
            }
            _ => Decoder::Identity,
        };
        EncodedPayload {
            inner: inner,
            decoder: dec,
            error: false,
            dst: BytesMut::new().writer(),
        }
    }
}

impl PayloadWriter for EncodedPayload {

    fn set_error(&mut self, err: PayloadError) {
        self.inner.set_error(err)
    }

    fn feed_eof(&mut self) {
        if self.error {
            return
        }
        let err = match self.decoder {
            Decoder::Br(ref mut buf, ref mut decoder) => {
                match decoder.flush() {
                    Ok(_) => {
                        let b = buf.borrow_mut().take().freeze();
                        if !b.is_empty() {
                            self.inner.feed_data(b);
                        }
                        self.inner.feed_eof();
                        return
                    },
                    Err(err) => Some(err),
                }
            }

            Decoder::Gzip(ref mut decoder) => {
                if decoder.is_none() {
                    self.inner.feed_eof();
                    return
                }
                loop {
                    let len = self.dst.get_ref().len();
                    let len_buf = decoder.as_mut().unwrap().get_mut().buf.len();

                    if len < len_buf * 2 {
                        self.dst.get_mut().reserve(len_buf * 2 - len);
                        unsafe{self.dst.get_mut().set_len(len_buf * 2)};
                    }
                    match decoder.as_mut().unwrap().read(&mut self.dst.get_mut()) {
                        Ok(n) =>  {
                            if n == 0 {
                                self.inner.feed_eof();
                                return
                            } else {
                                self.inner.feed_data(self.dst.get_mut().split_to(n).freeze());
                            }
                        }
                        Err(err) => break Some(err)
                    }
                }
            }
            Decoder::Zlib(ref mut decoder) => {
                match decoder.flush() {
                    Ok(_) => {
                        let b = decoder.get_mut().buf.take().freeze();
                        if !b.is_empty() {
                            self.inner.feed_data(b);
                        }
                        self.inner.feed_eof();
                        return
                    },
                    Err(err) => Some(err),
                }
            },
            Decoder::Identity => {
                self.inner.feed_eof();
                return
            }
        };

        self.error = true;
        self.decoder = Decoder::Identity;
        if let Some(err) = err {
            self.set_error(PayloadError::ParseError(err));
        } else {
            self.set_error(PayloadError::Incomplete);
        }
    }

    fn feed_data(&mut self, data: Bytes) {
        if self.error {
            return
        }
        match self.decoder {
            Decoder::Br(ref mut buf, ref mut decoder) => {
                match decoder.write(&data) {
                    Ok(_) => {
                        let b = buf.borrow_mut().take().freeze();
                        if !b.is_empty() {
                            self.inner.feed_data(b);
                        }
                        return
                    },
                    Err(err) => {
                        trace!("Error decoding br encoding: {}", err);
                    },
                }
            }

            Decoder::Gzip(ref mut decoder) => {
                if decoder.is_none() {
                    let mut buf = BytesMut::new();
                    buf.extend(data);
                    *decoder = Some(GzDecoder::new(Wrapper{buf: buf}).unwrap());
                } else {
                    decoder.as_mut().unwrap().get_mut().buf.extend(data);
                }

                loop {
                    let len_buf = decoder.as_mut().unwrap().get_mut().buf.len();
                    if len_buf == 0 {
                        return
                    }

                    let len = self.dst.get_ref().len();
                    if len < len_buf * 2 {
                        self.dst.get_mut().reserve(len_buf * 2 - len);
                        unsafe{self.dst.get_mut().set_len(len_buf * 2)};
                    }
                    match decoder.as_mut().unwrap().read(&mut self.dst.get_mut()) {
                        Ok(n) =>  {
                            if n == 0 {
                                return
                            } else {
                                self.inner.feed_data(self.dst.get_mut().split_to(n).freeze());
                            }
                        }
                        Err(_) => break
                    }
                }
            }

            Decoder::Zlib(ref mut decoder) => {
                match decoder.write(&data) {
                    Ok(_) => {
                        let b = decoder.get_mut().buf.take().freeze();
                        if !b.is_empty() {
                            self.inner.feed_data(b);
                        }
                        return
                    },
                    Err(err) => {
                        trace!("Error decoding deflate encoding: {}", err);
                    },
                }
            }
            Decoder::Identity => {
                self.inner.feed_data(data);
                return
            }
        };

        self.error = true;
        self.decoder = Decoder::Identity;
        self.set_error(PayloadError::EncodingCorrupted);
    }

    fn capacity(&self) -> usize {
        match self.decoder {
            Decoder::Br(ref buf, _) => {
                buf.borrow().len() + self.inner.capacity()
            }
            _ => {
                self.inner.capacity()
            }
        }
    }
}

#[derive(Debug)]
struct Inner {
    len: usize,
    eof: bool,
    err: Option<PayloadError>,
    task: Option<Task>,
    items: VecDeque<Bytes>,
    buf_size: usize,
}

impl Inner {

    fn new(eof: bool) -> Self {
        Inner {
            len: 0,
            eof: eof,
            err: None,
            task: None,
            items: VecDeque::new(),
            buf_size: DEFAULT_BUFFER_SIZE,
        }
    }

    fn set_error(&mut self, err: PayloadError) {
        self.err = Some(err);
        if let Some(task) = self.task.take() {
            task.notify()
        }
    }

    fn feed_eof(&mut self) {
        self.eof = true;
        if let Some(task) = self.task.take() {
            task.notify()
        }
    }

    fn feed_data(&mut self, data: Bytes) {
        self.len += data.len();
        self.items.push_back(data);
        if let Some(task) = self.task.take() {
            task.notify()
        }
    }

    fn eof(&self) -> bool {
        self.items.is_empty() && self.eof
    }

    fn len(&self) -> usize {
        self.len
    }

    fn readany(&mut self) -> Poll<Option<PayloadItem>, PayloadError> {
        if let Some(data) = self.items.pop_front() {
            self.len -= data.len();
            Ok(Async::Ready(Some(PayloadItem(data))))
        } else if self.eof {
            Ok(Async::Ready(None))
        } else if let Some(err) = self.err.take() {
            Err(err)
        } else {
            self.task = Some(current_task());
            Ok(Async::NotReady)
        }
    }

    fn readexactly(&mut self, size: usize) -> Result<Async<Bytes>, PayloadError> {
        if size <= self.len {
            let mut buf = BytesMut::with_capacity(size);
            while buf.len() < size {
                let mut chunk = self.items.pop_front().unwrap();
                let rem = cmp::min(size - buf.len(), chunk.len());
                self.len -= rem;
                buf.extend(&chunk.split_to(rem));
                if !chunk.is_empty() {
                    self.items.push_front(chunk);
                    return Ok(Async::Ready(buf.freeze()))
                }
            }
        }

        if let Some(err) = self.err.take() {
            Err(err)
        } else {
            self.task = Some(current_task());
            Ok(Async::NotReady)
        }
    }

    fn readuntil(&mut self, line: &[u8]) -> Result<Async<Bytes>, PayloadError> {
        let mut idx = 0;
        let mut num = 0;
        let mut offset = 0;
        let mut found = false;
        let mut length = 0;

        for no in 0..self.items.len() {
            {
                let chunk = &self.items[no];
                for (pos, ch) in chunk.iter().enumerate() {
                    if *ch == line[idx] {
                        idx += 1;
                        if idx == line.len() {
                            num = no;
                            offset = pos+1;
                            length += pos+1;
                            found = true;
                            break;
                        }
                    } else {
                        idx = 0
                    }
                }
                if !found {
                    length += chunk.len()
                }
            }

            if found {
                let mut buf = BytesMut::with_capacity(length);
                if num > 0 {
                    for _ in 0..num {
                        buf.extend(self.items.pop_front().unwrap());
                    }
                }
                if offset > 0 {
                    let mut chunk = self.items.pop_front().unwrap();
                    buf.extend(chunk.split_to(offset));
                    if !chunk.is_empty() {
                        self.items.push_front(chunk)
                    }
                }
                self.len -= length;
                return Ok(Async::Ready(buf.freeze()))
            }
        }
        if let Some(err) = self.err.take() {
            Err(err)
        } else {
            self.task = Some(current_task());
            Ok(Async::NotReady)
        }
    }

    fn readline(&mut self) -> Result<Async<Bytes>, PayloadError> {
        self.readuntil(b"\n")
    }

    pub fn readall(&mut self) -> Option<Bytes> {
        let len = self.items.iter().fold(0, |cur, item| cur + item.len());
        if len > 0 {
            let mut buf = BytesMut::with_capacity(len);
            for item in &self.items {
                buf.extend(item);
            }
            self.items = VecDeque::new();
            self.len = 0;
            Some(buf.take().freeze())
        } else {
            None
        }
    }

    fn unread_data(&mut self, data: Bytes) {
        self.len += data.len();
        self.items.push_front(data)
    }

    fn capacity(&self) -> usize {
        if self.len > self.buf_size {
            0
        } else {
            self.buf_size - self.len
        }
    }

    fn buffer_size(&self) -> usize {
        self.buf_size
    }

    fn set_buffer_size(&mut self, size: usize) {
        self.buf_size = size
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use futures::future::{lazy, result};
    use tokio_core::reactor::Core;

    #[test]
    fn test_error() {
        let err: PayloadError = IoError::new(io::ErrorKind::Other, "ParseError").into();
        assert_eq!(err.description(), "ParseError");
        assert_eq!(err.cause().unwrap().description(), "ParseError");
        assert_eq!(format!("{}", err), "ParseError");

        let err = PayloadError::Incomplete;
        assert_eq!(err.description(), "A payload reached EOF, but is not complete.");
        assert_eq!(format!("{}", err), "A payload reached EOF, but is not complete.");
    }

    #[test]
    fn test_basic() {
        Core::new().unwrap().run(lazy(|| {
            let (_, mut payload) = Payload::new(false);

            assert!(!payload.eof());
            assert!(payload.is_empty());
            assert_eq!(payload.len(), 0);

            match payload.readany() {
                Ok(Async::NotReady) => (),
                _ => panic!("error"),
            }

            let res: Result<(), ()> = Ok(());
            result(res)
        })).unwrap();
    }

    #[test]
    fn test_eof() {
        Core::new().unwrap().run(lazy(|| {
            let (mut sender, mut payload) = Payload::new(false);

            match payload.readany() {
                Ok(Async::NotReady) => (),
                _ => panic!("error"),
            }

            assert!(!payload.eof());

            sender.feed_data(Bytes::from("data"));
            sender.feed_eof();

            assert!(!payload.eof());

            match payload.readany() {
                Ok(Async::Ready(Some(data))) => assert_eq!(&data.0, "data"),
                _ => panic!("error"),
            }
            assert!(payload.is_empty());
            assert!(payload.eof());
            assert_eq!(payload.len(), 0);

            match payload.readany() {
                Ok(Async::Ready(None)) => (),
                _ => panic!("error"),
            }
            let res: Result<(), ()> = Ok(());
            result(res)
        })).unwrap();
    }

    #[test]
    fn test_err() {
        Core::new().unwrap().run(lazy(|| {
            let (mut sender, mut payload) = Payload::new(false);

            match payload.readany() {
                Ok(Async::NotReady) => (),
                _ => panic!("error"),
            }

            sender.set_error(PayloadError::Incomplete);
            match payload.readany() {
                Err(_) => (),
                _ => panic!("error"),
            }
            let res: Result<(), ()> = Ok(());
            result(res)
        })).unwrap();
    }

    #[test]
    fn test_readany() {
        Core::new().unwrap().run(lazy(|| {
            let (mut sender, mut payload) = Payload::new(false);

            sender.feed_data(Bytes::from("line1"));

            assert!(!payload.is_empty());
            assert_eq!(payload.len(), 5);

            sender.feed_data(Bytes::from("line2"));
            assert!(!payload.is_empty());
            assert_eq!(payload.len(), 10);

            match payload.readany() {
                Ok(Async::Ready(Some(data))) => assert_eq!(&data.0, "line1"),
                _ => panic!("error"),
            }
            assert!(!payload.is_empty());
            assert_eq!(payload.len(), 5);

            let res: Result<(), ()> = Ok(());
            result(res)
        })).unwrap();
    }

    #[test]
    fn test_readexactly() {
        Core::new().unwrap().run(lazy(|| {
            let (mut sender, mut payload) = Payload::new(false);

            match payload.readexactly(2) {
                Ok(Async::NotReady) => (),
                _ => panic!("error"),
            }

            sender.feed_data(Bytes::from("line1"));
            sender.feed_data(Bytes::from("line2"));
            assert_eq!(payload.len(), 10);

            match payload.readexactly(2) {
                Ok(Async::Ready(data)) => assert_eq!(&data, "li"),
                _ => panic!("error"),
            }
            assert_eq!(payload.len(), 8);

            match payload.readexactly(4) {
                Ok(Async::Ready(data)) => assert_eq!(&data, "ne1l"),
                _ => panic!("error"),
            }
            assert_eq!(payload.len(), 4);

            sender.set_error(PayloadError::Incomplete);
            match payload.readexactly(10) {
                Err(_) => (),
                _ => panic!("error"),
            }

            let res: Result<(), ()> = Ok(());
            result(res)
        })).unwrap();
    }

    #[test]
    fn test_readuntil() {
        Core::new().unwrap().run(lazy(|| {
            let (mut sender, mut payload) = Payload::new(false);

            match payload.readuntil(b"ne") {
                Ok(Async::NotReady) => (),
                _ => panic!("error"),
            }

            sender.feed_data(Bytes::from("line1"));
            sender.feed_data(Bytes::from("line2"));
            assert_eq!(payload.len(), 10);

            match payload.readuntil(b"ne") {
                Ok(Async::Ready(data)) => assert_eq!(&data, "line"),
                _ => panic!("error"),
            }
            assert_eq!(payload.len(), 6);

            match payload.readuntil(b"2") {
                Ok(Async::Ready(data)) => assert_eq!(&data, "1line2"),
                _ => panic!("error"),
            }
            assert_eq!(payload.len(), 0);

            sender.set_error(PayloadError::Incomplete);
            match payload.readuntil(b"b") {
                Err(_) => (),
                _ => panic!("error"),
            }

            let res: Result<(), ()> = Ok(());
            result(res)
        })).unwrap();
    }

    #[test]
    fn test_unread_data() {
        Core::new().unwrap().run(lazy(|| {
            let (_, mut payload) = Payload::new(false);

            payload.unread_data(Bytes::from("data"));
            assert!(!payload.is_empty());
            assert_eq!(payload.len(), 4);

            match payload.readany() {
                Ok(Async::Ready(Some(data))) => assert_eq!(&data.0, "data"),
                _ => panic!("error"),
            }

            let res: Result<(), ()> = Ok(());
            result(res)
        })).unwrap();
    }
}
