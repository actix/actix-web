use futures::{Async, Poll};

use super::{helpers, HttpHandlerTask, Writer};
use http::{StatusCode, Version};
use Error;

pub(crate) struct ServerError(Version, StatusCode);

impl ServerError {
    pub fn err(ver: Version, status: StatusCode) -> Box<HttpHandlerTask> {
        Box::new(ServerError(ver, status))
    }
}

impl HttpHandlerTask for ServerError {
    fn poll_io(&mut self, io: &mut Writer) -> Poll<bool, Error> {
        {
            let bytes = io.buffer();
            // Buffer should have sufficient capacity for status line
            // and extra space
            bytes.reserve(helpers::STATUS_LINE_BUF_SIZE + 1);
            helpers::write_status_line(self.0, self.1.as_u16(), bytes);
        }
        io.buffer().extend_from_slice(b"\r\ncontent-length: 0\r\n");
        io.set_date();
        Ok(Async::Ready(true))
    }
}
