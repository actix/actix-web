use futures::{Async, Poll};

use super::{helpers, HttpHandlerTask, Writer};
use http::{StatusCode, Version};
use httpresponse::HttpResponse;
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
            let mut bytes = io.buffer();
            helpers::write_status_line(self.0, self.1.as_u16(), bytes);
        }
        io.set_date();
        Ok(Async::Ready(true))
    }
}
