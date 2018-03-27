use std::rc::Rc;
use std::cell::UnsafeCell;
use std::marker::PhantomData;
use serde::de::DeserializeOwned;
use futures::{Async, Future, Poll};

use error::Error;
use handler::{Handler, Reply, ReplyItem, Responder};
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use extractor::HttpRequestExtractor;


/// Trait defines object that could be registered as route handler
#[allow(unused_variables)]
pub trait WithHandler<T, D, S>: 'static
    where D: HttpRequestExtractor<T>, T: DeserializeOwned
{
    /// The type of value that handler will return.
    type Result: Responder;

    /// Handle request
    fn handle(&mut self, req: &HttpRequest<S>, data: D) -> Self::Result;
}

/// WithHandler<D, T, S> for Fn()
impl<T, D, S, F, R> WithHandler<T, D, S> for F
    where F: Fn(&HttpRequest<S>, D) -> R + 'static,
          R: Responder + 'static,
          D: HttpRequestExtractor<T>,
          T: DeserializeOwned,
{
    type Result = R;

    fn handle(&mut self, req: &HttpRequest<S>, item: D) -> R {
        (self)(req, item)
    }
}

pub fn with<T, D, S, H>(h: H) -> With<T, D, S, H>
    where H: WithHandler<T, D, S>,
          D: HttpRequestExtractor<T>,
          T: DeserializeOwned,
{
    With{hnd: Rc::new(UnsafeCell::new(h)),
         _t: PhantomData, _d: PhantomData, _s: PhantomData}
}

pub struct With<T, D, S, H>
    where H: WithHandler<T, D, S>,
          D: HttpRequestExtractor<T>,
          T: DeserializeOwned,
{
    hnd: Rc<UnsafeCell<H>>,
    _t: PhantomData<T>,
    _d: PhantomData<D>,
    _s: PhantomData<S>,
}

impl<T, D, S, H> Handler<S> for With<T, D, S, H>
    where H: WithHandler<T, D, S>,
          D: HttpRequestExtractor<T>,
          T: DeserializeOwned,
          T: 'static, D: 'static, S: 'static
{
    type Result = Reply;

    fn handle(&mut self, req: HttpRequest<S>) -> Self::Result {
        let fut = Box::new(D::extract(&req));

        Reply::async(
            WithHandlerFut{
                req,
                hnd: Rc::clone(&self.hnd),
                fut1: Some(fut),
                fut2: None,
                _t: PhantomData,
                _d: PhantomData,
            })
    }
}

struct WithHandlerFut<T, D, S, H>
    where H: WithHandler<T, D, S>,
          D: HttpRequestExtractor<T>,
          T: DeserializeOwned,
          T: 'static, D: 'static, S: 'static
{
    hnd: Rc<UnsafeCell<H>>,
    req: HttpRequest<S>,
    fut1: Option<Box<Future<Item=D, Error=Error>>>,
    fut2: Option<Box<Future<Item=HttpResponse, Error=Error>>>,
    _t: PhantomData<T>,
    _d: PhantomData<D>,
}

impl<T, D, S, H> Future for WithHandlerFut<T, D, S, H>
    where H: WithHandler<T, D, S>,
          D: HttpRequestExtractor<T>,
          T: DeserializeOwned,
          T: 'static, D: 'static, S: 'static
{
    type Item = HttpResponse;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut2 {
            return fut.poll()
        }

        let item = match self.fut1.as_mut().unwrap().poll()? {
            Async::Ready(item) => item,
            Async::NotReady => return Ok(Async::NotReady),
        };

        let hnd: &mut H = unsafe{&mut *self.hnd.get()};
        let item = match hnd.handle(&self.req, item).respond_to(self.req.without_state())
        {
            Ok(item) => item.into(),
            Err(err) => return Err(err.into()),
        };

        match item.into() {
            ReplyItem::Message(resp) => return Ok(Async::Ready(resp)),
            ReplyItem::Future(fut) => self.fut2 = Some(fut),
        }

        self.poll()
    }
}
