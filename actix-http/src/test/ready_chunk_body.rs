use std::{
    cell::Cell,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use bytes::Bytes;

use crate::{body::MessageBody, Error};

/// Test body that yields fixed-size chunks for a configured number of polls.
pub(crate) struct ReadyChunkBody {
    chunk_polls: Rc<Cell<usize>>,
    remaining: usize,
    chunk_len: usize,
}

impl ReadyChunkBody {
    pub(crate) fn new(chunk_polls: Rc<Cell<usize>>, remaining: usize, chunk_len: usize) -> Self {
        Self {
            chunk_polls,
            remaining,
            chunk_len,
        }
    }
}

impl MessageBody for ReadyChunkBody {
    type Error = Error;

    fn size(&self) -> crate::body::BodySize {
        crate::body::BodySize::Stream
    }

    fn poll_next(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        if self.remaining == 0 {
            return Poll::Ready(None);
        }

        self.remaining -= 1;
        self.chunk_polls.set(self.chunk_polls.get() + 1);

        Poll::Ready(Some(Ok(Bytes::from(vec![b'x'; self.chunk_len]))))
    }
}

#[cfg(test)]
mod tests {
    use std::task::Context;

    use futures_util::task::noop_waker_ref;

    use super::*;

    #[test]
    fn yields_configured_chunks() {
        let chunk_polls = Rc::new(Cell::new(0));
        let mut body = ReadyChunkBody::new(chunk_polls.clone(), 2, 3);
        let mut cx = Context::from_waker(noop_waker_ref());

        assert_eq!(body.size(), crate::body::BodySize::Stream);
        assert_eq!(
            Pin::new(&mut body)
                .poll_next(&mut cx)
                .map(|chunk| chunk.unwrap().unwrap()),
            Poll::Ready(Bytes::from_static(b"xxx"))
        );
        assert_eq!(
            Pin::new(&mut body)
                .poll_next(&mut cx)
                .map(|chunk| chunk.unwrap().unwrap()),
            Poll::Ready(Bytes::from_static(b"xxx"))
        );
        assert_eq!(
            Pin::new(&mut body)
                .poll_next(&mut cx)
                .map(|chunk| chunk.is_none()),
            Poll::Ready(true)
        );
        assert_eq!(chunk_polls.get(), 2);
    }
}
