use std::{
    cmp, fmt,
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use actix_web::{error::Error, web::Bytes};
#[cfg(feature = "experimental-io-uring")]
use bytes::BytesMut;
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

use super::named::File;

pin_project! {
    /// Adapter to read a `std::file::File` in chunks.
    #[doc(hidden)]
    pub struct ChunkedReadFile<F, Fut> {
        size: u64,
        offset: u64,
        #[pin]
        state: ChunkedReadFileState<Fut>,
        counter: u64,
        callback: F,
    }
}

#[cfg(not(feature = "experimental-io-uring"))]
pin_project! {
    #[project = ChunkedReadFileStateProj]
    #[project_replace = ChunkedReadFileStateProjReplace]
    enum ChunkedReadFileState<Fut> {
        File { file: Option<File>, },
        Future { #[pin] fut: Fut },
    }
}

#[cfg(feature = "experimental-io-uring")]
pin_project! {
    #[project = ChunkedReadFileStateProj]
    #[project_replace = ChunkedReadFileStateProjReplace]
    enum ChunkedReadFileState<Fut> {
        File { file: Option<(File, BytesMut)> },
        Future { #[pin] fut: Fut },
    }
}

impl<F, Fut> fmt::Debug for ChunkedReadFile<F, Fut> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ChunkedReadFile")
    }
}

pub(crate) fn new_chunked_read(
    size: u64,
    offset: u64,
    file: File,
) -> impl Stream<Item = Result<Bytes, Error>> {
    ChunkedReadFile {
        size,
        offset,
        #[cfg(not(feature = "experimental-io-uring"))]
        state: ChunkedReadFileState::File { file: Some(file) },
        #[cfg(feature = "experimental-io-uring")]
        state: ChunkedReadFileState::File {
            file: Some((file, BytesMut::new())),
        },
        counter: 0,
        callback: chunked_read_file_callback,
    }
}

#[cfg(not(feature = "experimental-io-uring"))]
async fn chunked_read_file_callback(
    mut file: File,
    offset: u64,
    max_bytes: usize,
) -> Result<(File, Bytes), Error> {
    use io::{Read as _, Seek as _};

    let res = actix_web::web::block(move || {
        let mut buf = Vec::with_capacity(max_bytes);

        file.seek(io::SeekFrom::Start(offset))?;

        let n_bytes = file.by_ref().take(max_bytes as u64).read_to_end(&mut buf)?;

        if n_bytes == 0 {
            Err(io::Error::from(io::ErrorKind::UnexpectedEof))
        } else {
            Ok((file, Bytes::from(buf)))
        }
    })
    .await??;

    Ok(res)
}

#[cfg(feature = "experimental-io-uring")]
async fn chunked_read_file_callback(
    file: File,
    offset: u64,
    max_bytes: usize,
    mut bytes_mut: BytesMut,
) -> io::Result<(File, Bytes, BytesMut)> {
    bytes_mut.reserve(max_bytes);

    let (res, mut bytes_mut) = file.read_at(bytes_mut, offset).await;
    let n_bytes = res?;

    if n_bytes == 0 {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }

    let bytes = bytes_mut.split_to(n_bytes).freeze();

    Ok((file, bytes, bytes_mut))
}

#[cfg(feature = "experimental-io-uring")]
impl<F, Fut> Stream for ChunkedReadFile<F, Fut>
where
    F: Fn(File, u64, usize, BytesMut) -> Fut,
    Fut: Future<Output = io::Result<(File, Bytes, BytesMut)>>,
{
    type Item = Result<Bytes, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.as_mut().project();
        match this.state.as_mut().project() {
            ChunkedReadFileStateProj::File { file } => {
                let size = *this.size;
                let offset = *this.offset;
                let counter = *this.counter;

                if size == counter {
                    Poll::Ready(None)
                } else {
                    let max_bytes = cmp::min(size.saturating_sub(counter), 65_536) as usize;

                    let (file, bytes_mut) = file
                        .take()
                        .expect("ChunkedReadFile polled after completion");

                    let fut = (this.callback)(file, offset, max_bytes, bytes_mut);

                    this.state
                        .project_replace(ChunkedReadFileState::Future { fut });

                    self.poll_next(cx)
                }
            }
            ChunkedReadFileStateProj::Future { fut } => {
                let (file, bytes, bytes_mut) = ready!(fut.poll(cx))?;

                this.state.project_replace(ChunkedReadFileState::File {
                    file: Some((file, bytes_mut)),
                });

                *this.offset += bytes.len() as u64;
                *this.counter += bytes.len() as u64;

                Poll::Ready(Some(Ok(bytes)))
            }
        }
    }
}

#[cfg(not(feature = "experimental-io-uring"))]
impl<F, Fut> Stream for ChunkedReadFile<F, Fut>
where
    F: Fn(File, u64, usize) -> Fut,
    Fut: Future<Output = Result<(File, Bytes), Error>>,
{
    type Item = Result<Bytes, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.as_mut().project();
        match this.state.as_mut().project() {
            ChunkedReadFileStateProj::File { file } => {
                let size = *this.size;
                let offset = *this.offset;
                let counter = *this.counter;

                if size == counter {
                    Poll::Ready(None)
                } else {
                    let max_bytes = cmp::min(size.saturating_sub(counter), 65_536) as usize;

                    let file = file
                        .take()
                        .expect("ChunkedReadFile polled after completion");

                    let fut = (this.callback)(file, offset, max_bytes);

                    this.state
                        .project_replace(ChunkedReadFileState::Future { fut });

                    self.poll_next(cx)
                }
            }
            ChunkedReadFileStateProj::Future { fut } => {
                let (file, bytes) = ready!(fut.poll(cx))?;

                this.state
                    .project_replace(ChunkedReadFileState::File { file: Some(file) });

                *this.offset += bytes.len() as u64;
                *this.counter += bytes.len() as u64;

                Poll::Ready(Some(Ok(bytes)))
            }
        }
    }
}
