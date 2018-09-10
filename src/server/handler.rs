use futures::{Async, Poll};

use super::message::Request;
use super::Writer;
use error::Error;

/// Low level http request handler
#[allow(unused_variables)]
pub trait HttpHandler: 'static {
    /// Request handling task
    type Task: HttpHandlerTask;

    /// Handle request
    fn handle(&self, req: Request) -> Result<Self::Task, Request>;
}

impl HttpHandler for Box<HttpHandler<Task = Box<HttpHandlerTask>>> {
    type Task = Box<HttpHandlerTask>;

    fn handle(&self, req: Request) -> Result<Box<HttpHandlerTask>, Request> {
        self.as_ref().handle(req)
    }
}

/// Low level http request handler
pub trait HttpHandlerTask {
    /// Poll task, this method is used before or after *io* object is available
    fn poll_completed(&mut self) -> Poll<(), Error> {
        Ok(Async::Ready(()))
    }

    /// Poll task when *io* object is available
    fn poll_io(&mut self, io: &mut Writer) -> Poll<bool, Error>;

    /// Connection is disconnected
    fn disconnected(&mut self) {}
}

impl HttpHandlerTask for Box<HttpHandlerTask> {
    fn poll_io(&mut self, io: &mut Writer) -> Poll<bool, Error> {
        self.as_mut().poll_io(io)
    }
}

/// Conversion helper trait
pub trait IntoHttpHandler {
    /// The associated type which is result of conversion.
    type Handler: HttpHandler;

    /// Convert into `HttpHandler` object.
    fn into_handler(self) -> Self::Handler;
}

impl<T: HttpHandler> IntoHttpHandler for T {
    type Handler = T;

    fn into_handler(self) -> Self::Handler {
        self
    }
}

impl<T: IntoHttpHandler> IntoHttpHandler for Vec<T> {
    type Handler = VecHttpHandler<T::Handler>;

    fn into_handler(self) -> Self::Handler {
        VecHttpHandler(self.into_iter().map(|item| item.into_handler()).collect())
    }
}

#[doc(hidden)]
pub struct VecHttpHandler<H: HttpHandler>(Vec<H>);

impl<H: HttpHandler> HttpHandler for VecHttpHandler<H> {
    type Task = H::Task;

    fn handle(&self, mut req: Request) -> Result<Self::Task, Request> {
        for h in &self.0 {
            req = match h.handle(req) {
                Ok(task) => return Ok(task),
                Err(e) => e,
            };
        }
        Err(req)
    }
}

macro_rules! http_handler ({$EN:ident, $(($n:tt, $T:ident)),+} => {
    impl<$($T: HttpHandler,)+> HttpHandler for ($($T,)+) {
        type Task = $EN<$($T,)+>;

        fn handle(&self, mut req: Request) -> Result<Self::Task, Request> {
            $(
                req = match self.$n.handle(req) {
                    Ok(task) => return Ok($EN::$T(task)),
                    Err(e) => e,
                };
            )+
                Err(req)
        }
    }

    #[doc(hidden)]
    pub enum $EN<$($T: HttpHandler,)+> {
        $($T ($T::Task),)+
    }

    impl<$($T: HttpHandler,)+> HttpHandlerTask for $EN<$($T,)+>
    {
        fn poll_completed(&mut self) -> Poll<(), Error> {
            match self {
                $($EN :: $T(ref mut task) => task.poll_completed(),)+
            }
        }

        fn poll_io(&mut self, io: &mut Writer) -> Poll<bool, Error> {
            match self {
                $($EN::$T(ref mut task) => task.poll_io(io),)+
            }
        }

        /// Connection is disconnected
        fn disconnected(&mut self) {
            match self {
                $($EN::$T(ref mut task) => task.disconnected(),)+
            }
        }
    }
});

http_handler!(HttpHandlerTask1, (0, A));
http_handler!(HttpHandlerTask2, (0, A), (1, B));
http_handler!(HttpHandlerTask3, (0, A), (1, B), (2, C));
http_handler!(HttpHandlerTask4, (0, A), (1, B), (2, C), (3, D));
http_handler!(HttpHandlerTask5, (0, A), (1, B), (2, C), (3, D), (4, E));
http_handler!(
    HttpHandlerTask6,
    (0, A),
    (1, B),
    (2, C),
    (3, D),
    (4, E),
    (5, F)
);
http_handler!(
    HttpHandlerTask7,
    (0, A),
    (1, B),
    (2, C),
    (3, D),
    (4, E),
    (5, F),
    (6, G)
);
http_handler!(
    HttpHandlerTask8,
    (0, A),
    (1, B),
    (2, C),
    (3, D),
    (4, E),
    (5, F),
    (6, G),
    (7, H)
);
http_handler!(
    HttpHandlerTask9,
    (0, A),
    (1, B),
    (2, C),
    (3, D),
    (4, E),
    (5, F),
    (6, G),
    (7, H),
    (8, I)
);
http_handler!(
    HttpHandlerTask10,
    (0, A),
    (1, B),
    (2, C),
    (3, D),
    (4, E),
    (5, F),
    (6, G),
    (7, H),
    (8, I),
    (9, J)
);
