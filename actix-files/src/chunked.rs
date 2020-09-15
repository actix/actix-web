use std::{
    cmp,
    fs::File,
    future::Future,
    io::{self, Read, Seek},
    pin::Pin,
    task::{Context, Poll},
};

use actix_web::{
    error::{BlockingError, Error},
    web,
};
use bytes::Bytes;
use futures_core::Stream;
use futures_util::future::{FutureExt, LocalBoxFuture};

use crate::handle_error;

type ChunkedBoxFuture =
    LocalBoxFuture<'static, Result<(File, Bytes), BlockingError<io::Error>>>;

#[doc(hidden)]
/// A helper created from a `std::fs::File` which reads the file
/// chunk-by-chunk on a `ThreadPool`.
pub struct ChunkedReadFile {
    pub(crate) size: u64,
    pub(crate) offset: u64,
    pub(crate) file: Option<File>,
    pub(crate) fut: Option<ChunkedBoxFuture>,
    pub(crate) counter: u64,
}

impl Stream for ChunkedReadFile {
    type Item = Result<Bytes, Error>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        if let Some(ref mut fut) = self.fut {
            return match Pin::new(fut).poll(cx) {
                Poll::Ready(Ok((file, bytes))) => {
                    self.fut.take();
                    self.file = Some(file);
                    self.offset += bytes.len() as u64;
                    self.counter += bytes.len() as u64;
                    Poll::Ready(Some(Ok(bytes)))
                }
                Poll::Ready(Err(e)) => Poll::Ready(Some(Err(handle_error(e)))),
                Poll::Pending => Poll::Pending,
            };
        }

        let size = self.size;
        let offset = self.offset;
        let counter = self.counter;

        if size == counter {
            Poll::Ready(None)
        } else {
            let mut file = self.file.take().expect("Use after completion");
            self.fut = Some(
                web::block(move || {
                    let max_bytes: usize;
                    max_bytes = cmp::min(size.saturating_sub(counter), 65_536) as usize;
                    let mut buf = Vec::with_capacity(max_bytes);
                    file.seek(io::SeekFrom::Start(offset))?;
                    let nbytes =
                        file.by_ref().take(max_bytes as u64).read_to_end(&mut buf)?;
                    if nbytes == 0 {
                        return Err(io::ErrorKind::UnexpectedEof.into());
                    }
                    Ok((file, Bytes::from(buf)))
                })
                .boxed_local(),
            );
            self.poll_next(cx)
        }
    }
}
