#![allow(deprecated)]

use std::fmt;
use std::io::{self, Read, Write};

use bytes::BytesMut;
use futures::{Poll, Sink, StartSend, Stream};
use tokio_codec::{Decoder, Encoder};
use tokio_io::{AsyncRead, AsyncWrite};

use super::framed_read::{framed_read2, framed_read2_with_buffer, FramedRead2};
use super::framed_write::{framed_write2, framed_write2_with_buffer, FramedWrite2};

/// A unified `Stream` and `Sink` interface to an underlying I/O object, using
/// the `Encoder` and `Decoder` traits to encode and decode frames.
///
/// You can create a `Framed` instance by using the `AsyncRead::framed` adapter.
pub struct Framed2<T, D, E> {
    inner: FramedRead2<FramedWrite2<Fuse2<T, D, E>>>,
}

pub struct Fuse2<T, D, E>(pub T, pub D, pub E);

impl<T, D, E> Framed2<T, D, E>
where
    T: AsyncRead + AsyncWrite,
    D: Decoder,
    E: Encoder,
{
    /// Provides a `Stream` and `Sink` interface for reading and writing to this
    /// `Io` object, using `Decode` and `Encode` to read and write the raw data.
    ///
    /// Raw I/O objects work with byte sequences, but higher-level code usually
    /// wants to batch these into meaningful chunks, called "frames". This
    /// method layers framing on top of an I/O object, by using the `Codec`
    /// traits to handle encoding and decoding of messages frames. Note that
    /// the incoming and outgoing frame types may be distinct.
    ///
    /// This function returns a *single* object that is both `Stream` and
    /// `Sink`; grouping this into a single object is often useful for layering
    /// things like gzip or TLS, which require both read and write access to the
    /// underlying object.
    ///
    /// If you want to work more directly with the streams and sink, consider
    /// calling `split` on the `Framed` returned by this method, which will
    /// break them into separate objects, allowing them to interact more easily.
    pub fn new(inner: T, decoder: D, encoder: E) -> Framed2<T, D, E> {
        Framed2 {
            inner: framed_read2(framed_write2(Fuse2(inner, decoder, encoder))),
        }
    }
}

impl<T, D, E> Framed2<T, D, E> {
    /// Provides a `Stream` and `Sink` interface for reading and writing to this
    /// `Io` object, using `Decode` and `Encode` to read and write the raw data.
    ///
    /// Raw I/O objects work with byte sequences, but higher-level code usually
    /// wants to batch these into meaningful chunks, called "frames". This
    /// method layers framing on top of an I/O object, by using the `Codec`
    /// traits to handle encoding and decoding of messages frames. Note that
    /// the incoming and outgoing frame types may be distinct.
    ///
    /// This function returns a *single* object that is both `Stream` and
    /// `Sink`; grouping this into a single object is often useful for layering
    /// things like gzip or TLS, which require both read and write access to the
    /// underlying object.
    ///
    /// This objects takes a stream and a readbuffer and a writebuffer. These
    /// field can be obtained from an existing `Framed` with the
    /// `into_parts` method.
    ///
    /// If you want to work more directly with the streams and sink, consider
    /// calling `split` on the `Framed` returned by this method, which will
    /// break them into separate objects, allowing them to interact more easily.
    pub fn from_parts(parts: FramedParts2<T, D, E>) -> Framed2<T, D, E> {
        Framed2 {
            inner: framed_read2_with_buffer(
                framed_write2_with_buffer(
                    Fuse2(parts.io, parts.decoder, parts.encoder),
                    parts.write_buf,
                ),
                parts.read_buf,
            ),
        }
    }

    /// Returns a reference to the underlying I/O stream wrapped by
    /// `Frame`.
    ///
    /// Note that care should be taken to not tamper with the underlying stream
    /// of data coming in as it may corrupt the stream of frames otherwise
    /// being worked with.
    pub fn get_ref(&self) -> &T {
        &self.inner.get_ref().get_ref().0
    }

    /// Returns a mutable reference to the underlying I/O stream wrapped by
    /// `Frame`.
    ///
    /// Note that care should be taken to not tamper with the underlying stream
    /// of data coming in as it may corrupt the stream of frames otherwise
    /// being worked with.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner.get_mut().get_mut().0
    }

    /// Returns a reference to the underlying decoder.
    pub fn decocer(&self) -> &D {
        &self.inner.get_ref().get_ref().1
    }

    /// Returns a mutable reference to the underlying decoder.
    pub fn decoder_mut(&mut self) -> &mut D {
        &mut self.inner.get_mut().get_mut().1
    }

    /// Returns a reference to the underlying encoder.
    pub fn encoder(&self) -> &E {
        &self.inner.get_ref().get_ref().2
    }

    /// Returns a mutable reference to the underlying codec.
    pub fn encoder_mut(&mut self) -> &mut E {
        &mut self.inner.get_mut().get_mut().2
    }

    /// Consumes the `Frame`, returning its underlying I/O stream.
    ///
    /// Note that care should be taken to not tamper with the underlying stream
    /// of data coming in as it may corrupt the stream of frames otherwise
    /// being worked with.
    pub fn into_inner(self) -> T {
        self.inner.into_inner().into_inner().0
    }

    /// Consume the `Frame`, returning `Frame` with different codec.
    pub fn switch_encoder<E2>(self, encoder: E2) -> Framed2<T, D, E2> {
        let (inner, read_buf) = self.inner.into_parts();
        let (inner, write_buf) = inner.into_parts();

        Framed2 {
            inner: framed_read2_with_buffer(
                framed_write2_with_buffer(Fuse2(inner.0, inner.1, encoder), write_buf),
                read_buf,
            ),
        }
    }

    /// Consumes the `Frame`, returning its underlying I/O stream, the buffer
    /// with unprocessed data, and the codec.
    ///
    /// Note that care should be taken to not tamper with the underlying stream
    /// of data coming in as it may corrupt the stream of frames otherwise
    /// being worked with.
    pub fn into_parts(self) -> FramedParts2<T, D, E> {
        let (inner, read_buf) = self.inner.into_parts();
        let (inner, write_buf) = inner.into_parts();

        FramedParts2 {
            io: inner.0,
            decoder: inner.1,
            encoder: inner.2,
            read_buf: read_buf,
            write_buf: write_buf,
            _priv: (),
        }
    }
}

impl<T, D, E> Stream for Framed2<T, D, E>
where
    T: AsyncRead,
    D: Decoder,
{
    type Item = D::Item;
    type Error = D::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        self.inner.poll()
    }
}

impl<T, D, E> Sink for Framed2<T, D, E>
where
    T: AsyncWrite,
    E: Encoder,
    E::Error: From<io::Error>,
{
    type SinkItem = E::Item;
    type SinkError = E::Error;

    fn start_send(
        &mut self, item: Self::SinkItem,
    ) -> StartSend<Self::SinkItem, Self::SinkError> {
        self.inner.get_mut().start_send(item)
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        self.inner.get_mut().poll_complete()
    }

    fn close(&mut self) -> Poll<(), Self::SinkError> {
        self.inner.get_mut().close()
    }
}

impl<T, D, E> fmt::Debug for Framed2<T, D, E>
where
    T: fmt::Debug,
    D: fmt::Debug,
    E: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Framed2")
            .field("io", &self.inner.get_ref().get_ref().0)
            .field("decoder", &self.inner.get_ref().get_ref().1)
            .field("encoder", &self.inner.get_ref().get_ref().2)
            .finish()
    }
}

// ===== impl Fuse2 =====

impl<T: Read, D, E> Read for Fuse2<T, D, E> {
    fn read(&mut self, dst: &mut [u8]) -> io::Result<usize> {
        self.0.read(dst)
    }
}

impl<T: AsyncRead, D, E> AsyncRead for Fuse2<T, D, E> {
    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [u8]) -> bool {
        self.0.prepare_uninitialized_buffer(buf)
    }
}

impl<T: Write, D, E> Write for Fuse2<T, D, E> {
    fn write(&mut self, src: &[u8]) -> io::Result<usize> {
        self.0.write(src)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<T: AsyncWrite, D, E> AsyncWrite for Fuse2<T, D, E> {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.0.shutdown()
    }
}

impl<T, D: Decoder, E> Decoder for Fuse2<T, D, E> {
    type Item = D::Item;
    type Error = D::Error;

    fn decode(&mut self, buffer: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.1.decode(buffer)
    }

    fn decode_eof(&mut self, buffer: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.1.decode_eof(buffer)
    }
}

impl<T, D, E: Encoder> Encoder for Fuse2<T, D, E> {
    type Item = E::Item;
    type Error = E::Error;

    fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
        self.2.encode(item, dst)
    }
}

/// `FramedParts` contains an export of the data of a Framed transport.
/// It can be used to construct a new `Framed` with a different codec.
/// It contains all current buffers and the inner transport.
#[derive(Debug)]
pub struct FramedParts2<T, D, E> {
    /// The inner transport used to read bytes to and write bytes to
    pub io: T,

    /// The decoder
    pub decoder: D,

    /// The encoder
    pub encoder: E,

    /// The buffer with read but unprocessed data.
    pub read_buf: BytesMut,

    /// A buffer with unprocessed data which are not written yet.
    pub write_buf: BytesMut,

    /// This private field allows us to add additional fields in the future in a
    /// backwards compatible way.
    _priv: (),
}

impl<T, D, E> FramedParts2<T, D, E> {
    /// Create a new, default, `FramedParts`
    pub fn new(io: T, decoder: D, encoder: E) -> FramedParts2<T, D, E> {
        FramedParts2 {
            io,
            decoder,
            encoder,
            read_buf: BytesMut::new(),
            write_buf: BytesMut::new(),
            _priv: (),
        }
    }
}
