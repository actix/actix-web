use std::{
    cmp, fmt,
    fs::File,
    future::Future,
    io::{self, Read, Seek},
    pin::Pin,
    task::{Context, Poll},
};

use actix_web::{
    error::{BlockingError, Error},
    rt::task::{spawn_blocking, JoinHandle},
};
use bytes::Bytes;
use futures_core::{ready, Stream};

#[doc(hidden)]
/// A helper created from a `std::fs::File` which reads the file
/// chunk-by-chunk on a `ThreadPool`.
pub struct ChunkedReadFile {
    size: u64,
    offset: u64,
    state: ChunkedReadFileState,
    counter: u64,
}

enum ChunkedReadFileState {
    File(Option<File>),
    Future(JoinHandle<Result<(File, Bytes), io::Error>>),
}

impl ChunkedReadFile {
    pub(crate) fn new(size: u64, offset: u64, file: File) -> Self {
        Self {
            size,
            offset,
            state: ChunkedReadFileState::File(Some(file)),
            counter: 0,
        }
    }
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
        let this = self.as_mut().get_mut();
        match this.state {
            ChunkedReadFileState::File(ref mut file) => {
                let size = this.size;
                let offset = this.offset;
                let counter = this.counter;

                if size == counter {
                    Poll::Ready(None)
                } else {
                    let mut file = file
                        .take()
                        .expect("ChunkedReadFile polled after completion");

                    let fut = spawn_blocking(move || {
                        let max_bytes =
                            cmp::min(size.saturating_sub(counter), 65_536) as usize;

                        let mut buf = Vec::with_capacity(max_bytes);
                        file.seek(io::SeekFrom::Start(offset))?;

                        let n_bytes = file
                            .by_ref()
                            .take(max_bytes as u64)
                            .read_to_end(&mut buf)?;

                        if n_bytes == 0 {
                            return Err(io::ErrorKind::UnexpectedEof.into());
                        }

                        Ok((file, Bytes::from(buf)))
                    });
                    this.state = ChunkedReadFileState::Future(fut);
                    self.poll_next(cx)
                }
            }
            ChunkedReadFileState::Future(ref mut fut) => {
                let (file, bytes) =
                    ready!(Pin::new(fut).poll(cx)).map_err(|_| BlockingError)??;
                this.state = ChunkedReadFileState::File(Some(file));

                this.offset += bytes.len() as u64;
                this.counter += bytes.len() as u64;

                Poll::Ready(Some(Ok(bytes)))
            }
        }
    }
}
