use std::{
    cmp, fmt,
    fs::File,
    future::Future,
    io::{self, Read, Seek},
    pin::Pin,
    task::{Context, Poll},
};

use actix_web::{
    error::{Error, ErrorInternalServerError},
    rt::task::{spawn_blocking, JoinHandle},
};
use bytes::Bytes;
use futures_core::{ready, Stream};

#[doc(hidden)]
/// A helper created from a `std::fs::File` which reads the file
/// chunk-by-chunk on a `ThreadPool`.
pub struct ChunkedReadFile {
    pub(crate) size: u64,
    pub(crate) offset: u64,
    pub(crate) file: Option<File>,
    pub(crate) fut: Option<JoinHandle<Result<(File, Bytes), io::Error>>>,
    pub(crate) counter: u64,
}

impl fmt::Debug for ChunkedReadFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ChunkedReadFile")
    }
}

impl Stream for ChunkedReadFile {
    type Item = Result<Bytes, Error>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        if let Some(ref mut fut) = self.fut {
            let res = match ready!(Pin::new(fut).poll(cx)) {
                Ok(Ok((file, bytes))) => {
                    self.fut.take();
                    self.file = Some(file);

                    self.offset += bytes.len() as u64;
                    self.counter += bytes.len() as u64;

                    Ok(bytes)
                }
                Ok(Err(e)) => Err(e.into()),
                Err(_) => Err(ErrorInternalServerError("Unexpected error")),
            };
            return Poll::Ready(Some(res));
        }

        let size = self.size;
        let offset = self.offset;
        let counter = self.counter;

        if size == counter {
            Poll::Ready(None)
        } else {
            let mut file = self.file.take().expect("Use after completion");

            self.fut = Some(spawn_blocking(move || {
                let max_bytes = cmp::min(size.saturating_sub(counter), 65_536) as usize;

                let mut buf = Vec::with_capacity(max_bytes);
                file.seek(io::SeekFrom::Start(offset))?;

                let n_bytes =
                    file.by_ref().take(max_bytes as u64).read_to_end(&mut buf)?;

                if n_bytes == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }

                Ok((file, Bytes::from(buf)))
            }));

            self.poll_next(cx)
        }
    }
}
