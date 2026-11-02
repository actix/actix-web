use std::{
    fs::File,
    future::Future as _,
    io::{self, Read as _, Seek as _},
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use tokio::task::JoinHandle;

use super::{BodySize, MessageBody};

const CHUNK_SIZE: usize = 64 * 1024;

/// An owned file region that can also be read as a byte stream.
///
/// File contents must remain unchanged until the peer has consumed the response. The file's seek position
/// is not preserved. Constructors read metadata synchronously; body reads use Tokio's blocking pool
/// and require a Tokio runtime.
///
/// On macOS and Linux, the HTTP/1 dispatcher uses `sendfile` for plain Tokio TCP sockets. Other
/// transports read byte chunks through [`MessageBody::poll_next`]. Direct transfers can block on disk reads
/// when file data is not cached, even though the socket is nonblocking.
#[derive(Debug)]
pub struct FileBody {
    file: Option<File>,
    read: Option<JoinHandle<io::Result<(File, Bytes)>>>,
    offset: u64,
    remaining: u64,
    length: u64,
    direct: bool,
}

impl FileBody {
    /// Creates a body for the complete file, regardless of its current seek position.
    pub fn new(file: File) -> io::Result<Self> {
        let length = file.metadata()?.len();

        Self::region(file, 0, length)
    }

    /// Creates a body for a fixed region of a regular file.
    ///
    /// Returns an error if the region exceeds the current file length or the file is not regular.
    pub fn region(file: File, offset: u64, length: u64) -> io::Result<Self> {
        let metadata = file.metadata()?;

        if !metadata.is_file() || offset > metadata.len() || length > metadata.len() - offset {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid file region",
            ));
        }

        Ok(Self {
            file: Some(file),
            read: None,
            offset,
            remaining: length,
            length,
            direct: true,
        })
    }

    /// Disables direct file transfers and uses blocking-pool reads followed by socket writes.
    ///
    /// This is useful for comparing transfer methods with the same file and HTTP service.
    pub fn buffered(mut self) -> Self {
        self.direct = false;
        self
    }
}

/// Exclusive access to a body's remaining file region.
///
/// Use explicit offsets for transfers. Do not read through or change the seek position of `file()`.
/// Record only successfully transferred bytes with [`Self::advance`]. Dropping this handle does not
/// change the body's position. A transport must stop using the file before releasing this handle.
#[derive(Debug)]
pub struct FileRegion<'a> {
    file: &'a File,
    offset: &'a mut u64,
    remaining: &'a mut u64,
}

impl FileRegion<'_> {
    /// Returns the file to transfer from.
    pub fn file(&self) -> &File {
        self.file
    }

    /// Returns the offset of the next byte to transfer.
    pub fn offset(&self) -> u64 {
        *self.offset
    }

    /// Returns the number of bytes left in the region.
    pub fn remaining(&self) -> u64 {
        *self.remaining
    }

    /// Records successfully transferred bytes.
    ///
    /// # Panics
    /// Panics if `bytes` exceeds the remaining length.
    pub fn advance(&mut self, bytes: u64) {
        assert!(bytes <= *self.remaining, "transfer exceeds file region");

        *self.offset += bytes;
        *self.remaining -= bytes;
    }
}

impl MessageBody for FileBody {
    type Error = io::Error;

    fn size(&self) -> BodySize {
        BodySize::Sized(self.length)
    }

    fn file_region(self: Pin<&mut Self>) -> Option<FileRegion<'_>> {
        let this = self.get_mut();

        if !this.direct {
            return None;
        }

        Some(FileRegion {
            file: this.file.as_ref()?,
            offset: &mut this.offset,
            remaining: &mut this.remaining,
        })
    }

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        let this = self.get_mut();

        if this.remaining == 0 {
            return Poll::Ready(None);
        }

        if let Some(mut file) = this.file.take() {
            let offset = this.offset;
            let count = this.remaining.min(CHUNK_SIZE as u64) as usize;

            this.read = Some(tokio::task::spawn_blocking(move || {
                file.seek(io::SeekFrom::Start(offset))?;

                let mut buf = vec![0; count];
                let count = loop {
                    match file.read(&mut buf) {
                        Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                        result => break result?,
                    }
                };

                if count == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }

                buf.truncate(count);

                Ok((file, Bytes::from(buf)))
            }));
        }

        let result = match this.read.as_mut() {
            Some(read) => match Pin::new(read).poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(result) => result,
            },
            None => return Poll::Ready(None),
        };

        this.read = None;

        match result.map_err(io::Error::other).and_then(|result| result) {
            Ok((file, bytes)) => {
                this.offset += bytes.len() as u64;
                this.remaining -= bytes.len() as u64;
                this.file = Some(file);

                Poll::Ready(Some(Ok(bytes)))
            }
            Err(err) => {
                this.remaining = 0;

                Poll::Ready(Some(Err(err)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::Write as _,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use futures_util::future::poll_fn;

    use super::*;
    use crate::body::to_bytes;

    fn fixture() -> File {
        File::open(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap()
    }

    struct TestFile(std::path::PathBuf);

    impl TestFile {
        fn new(bytes: &[u8]) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "actix-http-file-body-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            let mut file = File::options()
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            file.write_all(bytes).unwrap();

            Self(path)
        }

        fn open(&self) -> File {
            File::open(&self.0).unwrap()
        }
    }

    impl Drop for TestFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[actix_rt::test]
    async fn read_then_transfer_then_read() {
        let bytes: Vec<_> = (0..CHUNK_SIZE * 2 + 17).map(|i| (i % 251) as u8).collect();
        let file = TestFile::new(&bytes);
        let mut body = FileBody::new(file.open()).unwrap();
        let first = poll_fn(|cx| {
            let result = Pin::new(&mut body).poll_next(cx);

            if result.is_pending() {
                assert!(Pin::new(&mut body).file_region().is_none());
            }

            result
        })
        .await
        .unwrap()
        .unwrap();

        assert_eq!(first.as_ref(), &bytes[..first.len()]);

        let mut region = Pin::new(&mut body).file_region().unwrap();
        assert_eq!(region.offset(), first.len() as u64);
        region.advance(11);

        assert_eq!(
            to_bytes(body).await.unwrap().as_ref(),
            &bytes[first.len() + 11..]
        );
    }

    #[actix_rt::test]
    async fn truncated_file_reports_error_and_stops() {
        let file = TestFile::new(b"content");
        let mut body = FileBody::new(file.open()).unwrap();
        File::options()
            .write(true)
            .open(&file.0)
            .unwrap()
            .set_len(0)
            .unwrap();

        let err = poll_fn(|cx| Pin::new(&mut body).poll_next(cx))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        assert!(poll_fn(|cx| Pin::new(&mut body).poll_next(cx))
            .await
            .is_none());
        assert!(Pin::new(&mut body).file_region().is_none());
    }

    #[actix_rt::test]
    async fn complete_file() {
        let body = FileBody::new(fixture()).unwrap();
        let expected = include_bytes!("../../Cargo.toml");

        assert_eq!(body.size(), BodySize::Sized(expected.len() as u64));
        assert_eq!(to_bytes(body).await.unwrap().as_ref(), expected);
    }

    #[actix_rt::test]
    async fn transfer_then_read_resumes_at_shared_offset() {
        let mut body = FileBody::region(fixture(), 2, 20).unwrap();

        {
            let mut region = Pin::new(&mut body).file_region().unwrap();
            assert_eq!(region.offset(), 2);
            assert_eq!(region.remaining(), 20);
            region.advance(7);
            assert_eq!(region.offset(), 9);
            assert_eq!(region.remaining(), 13);
        }

        assert_eq!(body.size(), BodySize::Sized(20));
        assert_eq!(
            to_bytes(body).await.unwrap().as_ref(),
            &include_bytes!("../../Cargo.toml")[9..22]
        );
    }

    #[actix_rt::test]
    async fn empty_region() {
        let length = fixture().metadata().unwrap().len();
        let body = FileBody::region(fixture(), length, 0).unwrap();

        assert!(to_bytes(body).await.unwrap().is_empty());
    }

    #[test]
    fn rejects_invalid_regions() {
        for (offset, length) in [(u64::MAX, 1), (1, u64::MAX)] {
            assert_eq!(
                FileBody::region(fixture(), offset, length)
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }

    #[test]
    #[should_panic(expected = "transfer exceeds file region")]
    fn rejects_excess_progress() {
        let mut body = FileBody::region(fixture(), 0, 1).unwrap();
        Pin::new(&mut body).file_region().unwrap().advance(2);
    }
}
