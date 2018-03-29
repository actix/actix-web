use std::rc::Rc;
use std::cell::UnsafeCell;
use std::marker::PhantomData;
use futures::{Async, Future, Poll};

use error::Error;
use handler::{Handler, FromRequest, Reply, ReplyItem, Responder};
use httprequest::HttpRequest;
use httpresponse::HttpResponse;


/// Trait defines object that could be registered as route handler
#[allow(unused_variables)]
pub trait WithHandler<T, S>: 'static
    where T: FromRequest<S>, S: 'static
{
    /// The type of value that handler will return.
    type Result: Responder;

    /// Handle request
    fn handle(&mut self, data: T) -> Self::Result;
}

/// WithHandler<D, T, S> for Fn()
impl<T, S, F, R> WithHandler<T, S> for F
    where F: Fn(T) -> R + 'static,
          R: Responder + 'static,
          T: FromRequest<S>,
          S: 'static,
{
    type Result = R;

    fn handle(&mut self, item: T) -> R {
        (self)(item)
    }
}

pub(crate)
fn with<T, S, H>(h: H) -> With<T, S, H>
    where H: WithHandler<T, S>,
          T: FromRequest<S>,
{
    With{hnd: Rc::new(UnsafeCell::new(h)), _t: PhantomData, _s: PhantomData}
}

pub struct With<T, S, H>
    where H: WithHandler<T, S> + 'static,
          T: FromRequest<S>,
          S: 'static,
{
    hnd: Rc<UnsafeCell<H>>,
    _t: PhantomData<T>,
    _s: PhantomData<S>,
}

impl<T, S, H> Handler<S> for With<T, S, H>
    where H: WithHandler<T, S>,
          T: FromRequest<S> + 'static,
          S: 'static, H: 'static
{
    type Result = Reply;

    fn handle(&mut self, req: HttpRequest<S>) -> Self::Result {
        let mut fut = WithHandlerFut{
            req,
            started: false,
            hnd: Rc::clone(&self.hnd),
            fut1: None,
            fut2: None,
        };

        match fut.poll() {
            Ok(Async::Ready(resp)) => Reply::response(resp),
            Ok(Async::NotReady) => Reply::async(fut),
            Err(e) => Reply::response(e),
        }
    }
}

struct WithHandlerFut<T, S, H>
    where H: WithHandler<T, S>,
          T: FromRequest<S>,
          T: 'static, S: 'static
{
    started: bool,
    hnd: Rc<UnsafeCell<H>>,
    req: HttpRequest<S>,
    fut1: Option<Box<Future<Item=T, Error=Error>>>,
    fut2: Option<Box<Future<Item=HttpResponse, Error=Error>>>,
}

impl<T, S, H> Future for WithHandlerFut<T, S, H>
    where H: WithHandler<T, S>,
          T: FromRequest<S> + 'static,
          S: 'static
{
    type Item = HttpResponse;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut2 {
            return fut.poll()
        }

        let item = if !self.started {
            self.started = true;
            let mut fut = T::from_request(&self.req);
            match fut.poll() {
                Ok(Async::Ready(item)) => item,
                Ok(Async::NotReady) => {
                    self.fut1 = Some(Box::new(fut));
                    return Ok(Async::NotReady)
                },
                Err(e) => return Err(e),
            }
        } else {
            match self.fut1.as_mut().unwrap().poll()? {
                Async::Ready(item) => item,
                Async::NotReady => return Ok(Async::NotReady),
            }
        };

        let hnd: &mut H = unsafe{&mut *self.hnd.get()};
        let item = match hnd.handle(item).respond_to(self.req.without_state()) {
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

pub(crate)
fn with2<T1, T2, S, F, R>(h: F) -> With2<T1, T2, S, F, R>
    where F: Fn(T1, T2) -> R,
          R: Responder,
          T1: FromRequest<S>,
          T2: FromRequest<S>,
{
    With2{hnd: Rc::new(UnsafeCell::new(h)),
          _t1: PhantomData, _t2: PhantomData, _s: PhantomData}
}

pub struct With2<T1, T2, S, F, R>
    where F: Fn(T1, T2) -> R,
          R: Responder,
          T1: FromRequest<S>,
          T2: FromRequest<S>,
          S: 'static,
{
    hnd: Rc<UnsafeCell<F>>,
    _t1: PhantomData<T1>,
    _t2: PhantomData<T2>,
    _s: PhantomData<S>,
}

impl<T1, T2, S, F, R> Handler<S> for With2<T1, T2, S, F, R>
    where F: Fn(T1, T2) -> R + 'static,
          R: Responder + 'static,
          T1: FromRequest<S> + 'static,
          T2: FromRequest<S> + 'static,
          S: 'static
{
    type Result = Reply;

    fn handle(&mut self, req: HttpRequest<S>) -> Self::Result {
        let mut fut = WithHandlerFut2{
            req,
            started: false,
            hnd: Rc::clone(&self.hnd),
            item: None,
            fut1: None,
            fut2: None,
            fut3: None,
        };
        match fut.poll() {
            Ok(Async::Ready(resp)) => Reply::response(resp),
            Ok(Async::NotReady) => Reply::async(fut),
            Err(e) => Reply::response(e),
        }
    }
}

struct WithHandlerFut2<T1, T2, S, F, R>
    where F: Fn(T1, T2) -> R + 'static,
          R: Responder + 'static,
          T1: FromRequest<S> + 'static,
          T2: FromRequest<S> + 'static,
          S: 'static
{
    started: bool,
    hnd: Rc<UnsafeCell<F>>,
    req: HttpRequest<S>,
    item: Option<T1>,
    fut1: Option<Box<Future<Item=T1, Error=Error>>>,
    fut2: Option<Box<Future<Item=T2, Error=Error>>>,
    fut3: Option<Box<Future<Item=HttpResponse, Error=Error>>>,
}

impl<T1, T2, S, F, R> Future for WithHandlerFut2<T1, T2, S, F, R>
    where F: Fn(T1, T2) -> R + 'static,
          R: Responder + 'static,
          T1: FromRequest<S> + 'static,
          T2: FromRequest<S> + 'static,
          S: 'static
{
    type Item = HttpResponse;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut3 {
            return fut.poll()
        }

        if !self.started {
            self.started = true;
            let mut fut = T1::from_request(&self.req);
            match fut.poll() {
                Ok(Async::Ready(item1)) => {
                    let mut fut = T2::from_request(&self.req);
                    match fut.poll() {
                        Ok(Async::Ready(item2)) => {
                            let hnd: &mut F = unsafe{&mut *self.hnd.get()};
                            match (*hnd)(item1, item2)
                                .respond_to(self.req.without_state())
                            {
                                Ok(item) => match item.into().into() {
                                    ReplyItem::Message(resp) =>
                                        return Ok(Async::Ready(resp)),
                                    ReplyItem::Future(fut) => {
                                        self.fut3 = Some(fut);
                                        return self.poll()
                                    }
                                },
                                Err(e) => return Err(e.into()),
                            }
                        },
                        Ok(Async::NotReady) => {
                            self.item = Some(item1);
                            self.fut2 = Some(Box::new(fut));
                            return Ok(Async::NotReady);
                        },
                        Err(e) => return Err(e),
                    }
                },
                Ok(Async::NotReady) => {
                    self.fut1 = Some(Box::new(fut));
                    return Ok(Async::NotReady);
                }
                Err(e) => return Err(e),
            }
        }

        if self.fut1.is_some() {
            match self.fut1.as_mut().unwrap().poll()? {
                Async::Ready(item) => {
                    self.item = Some(item);
                    self.fut1.take();
                    self.fut2 = Some(Box::new(T2::from_request(&self.req)));
                },
                Async::NotReady => return Ok(Async::NotReady),
            }
        }

        let item = match self.fut2.as_mut().unwrap().poll()? {
            Async::Ready(item) => item,
            Async::NotReady => return Ok(Async::NotReady),
        };

        let hnd: &mut F = unsafe{&mut *self.hnd.get()};
        let item = match (*hnd)(self.item.take().unwrap(), item)
            .respond_to(self.req.without_state())
        {
            Ok(item) => item.into(),
            Err(err) => return Err(err.into()),
        };

        match item.into() {
            ReplyItem::Message(resp) => return Ok(Async::Ready(resp)),
            ReplyItem::Future(fut) => self.fut3 = Some(fut),
        }

        self.poll()
    }
}

pub(crate)
fn with3<T1, T2, T3, S, F, R>(h: F) -> With3<T1, T2, T3, S, F, R>
    where F: Fn(T1, T2, T3) -> R + 'static,
          R: Responder,
          T1: FromRequest<S>,
          T2: FromRequest<S>,
          T3: FromRequest<S>,
{
    With3{hnd: Rc::new(UnsafeCell::new(h)),
          _s: PhantomData, _t1: PhantomData, _t2: PhantomData, _t3: PhantomData}
}

pub struct With3<T1, T2, T3, S, F, R>
    where F: Fn(T1, T2, T3) -> R + 'static,
          R: Responder + 'static,
          T1: FromRequest<S>,
          T2: FromRequest<S>,
          T3: FromRequest<S>,
          S: 'static,
{
    hnd: Rc<UnsafeCell<F>>,
    _t1: PhantomData<T1>,
    _t2: PhantomData<T2>,
    _t3: PhantomData<T3>,
    _s: PhantomData<S>,
}

impl<T1, T2, T3, S, F, R> Handler<S> for With3<T1, T2, T3, S, F, R>
    where F: Fn(T1, T2, T3) -> R + 'static,
          R: Responder + 'static,
          T1: FromRequest<S>,
          T2: FromRequest<S>,
          T3: FromRequest<S>,
          T1: 'static, T2: 'static, T3: 'static, S: 'static
{
    type Result = Reply;

    fn handle(&mut self, req: HttpRequest<S>) -> Self::Result {
        let mut fut = WithHandlerFut3{
            req,
            hnd: Rc::clone(&self.hnd),
            started: false,
            item1: None,
            item2: None,
            fut1: None,
            fut2: None,
            fut3: None,
            fut4: None,
        };
        match fut.poll() {
            Ok(Async::Ready(resp)) => Reply::response(resp),
            Ok(Async::NotReady) => Reply::async(fut),
            Err(e) => Reply::response(e),
        }
    }
}

struct WithHandlerFut3<T1, T2, T3, S, F, R>
    where F: Fn(T1, T2, T3) -> R + 'static,
          R: Responder + 'static,
          T1: FromRequest<S> + 'static,
          T2: FromRequest<S> + 'static,
          T3: FromRequest<S> + 'static,
          S: 'static
{
    hnd: Rc<UnsafeCell<F>>,
    req: HttpRequest<S>,
    started: bool,
    item1: Option<T1>,
    item2: Option<T2>,
    fut1: Option<Box<Future<Item=T1, Error=Error>>>,
    fut2: Option<Box<Future<Item=T2, Error=Error>>>,
    fut3: Option<Box<Future<Item=T3, Error=Error>>>,
    fut4: Option<Box<Future<Item=HttpResponse, Error=Error>>>,
}

impl<T1, T2, T3, S, F, R> Future for WithHandlerFut3<T1, T2, T3, S, F, R>
    where F: Fn(T1, T2, T3) -> R + 'static,
          R: Responder + 'static,
          T1: FromRequest<S> + 'static,
          T2: FromRequest<S> + 'static,
          T3: FromRequest<S> + 'static,
          S: 'static
{
    type Item = HttpResponse;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut4 {
            return fut.poll()
        }

        if !self.started {
            self.started = true;
            let mut fut = T1::from_request(&self.req);
            match fut.poll() {
                Ok(Async::Ready(item1)) => {
                    let mut fut = T2::from_request(&self.req);
                    match fut.poll() {
                        Ok(Async::Ready(item2)) => {
                            let mut fut = T3::from_request(&self.req);
                            match fut.poll() {
                                Ok(Async::Ready(item3)) => {
                                    let hnd: &mut F = unsafe{&mut *self.hnd.get()};
                                    match (*hnd)(item1, item2, item3)
                                        .respond_to(self.req.without_state())
                                    {
                                        Ok(item) => match item.into().into() {
                                            ReplyItem::Message(resp) =>
                                                return Ok(Async::Ready(resp)),
                                            ReplyItem::Future(fut) => {
                                                self.fut4 = Some(fut);
                                                return self.poll()
                                            }
                                        },
                                        Err(e) => return Err(e.into()),
                                    }
                                },
                                Ok(Async::NotReady) => {
                                    self.item1 = Some(item1);
                                    self.item2 = Some(item2);
                                    self.fut3 = Some(Box::new(fut));
                                    return Ok(Async::NotReady);
                                },
                                Err(e) => return Err(e),
                            }
                        },
                        Ok(Async::NotReady) => {
                            self.item1 = Some(item1);
                            self.fut2 = Some(Box::new(fut));
                            return Ok(Async::NotReady);
                        },
                        Err(e) => return Err(e),
                    }
                },
                Ok(Async::NotReady) => {
                    self.fut1 = Some(Box::new(fut));
                    return Ok(Async::NotReady);
                }
                Err(e) => return Err(e),
            }
        }

        if self.fut1.is_some() {
            match self.fut1.as_mut().unwrap().poll()? {
                Async::Ready(item) => {
                    self.item1 = Some(item);
                    self.fut1.take();
                    self.fut2 = Some(Box::new(T2::from_request(&self.req)));
                },
                Async::NotReady => return Ok(Async::NotReady),
            }
        }

        if self.fut2.is_some() {
            match self.fut2.as_mut().unwrap().poll()? {
                Async::Ready(item) => {
                    self.item2 = Some(item);
                    self.fut2.take();
                    self.fut3 = Some(Box::new(T3::from_request(&self.req)));
                },
                Async::NotReady => return Ok(Async::NotReady),
            }
        }

        let item = match self.fut3.as_mut().unwrap().poll()? {
            Async::Ready(item) => item,
            Async::NotReady => return Ok(Async::NotReady),
        };

        let hnd: &mut F = unsafe{&mut *self.hnd.get()};
        let item = match (*hnd)(self.item1.take().unwrap(),
                                self.item2.take().unwrap(),
                                item)
            .respond_to(self.req.without_state())
        {
            Ok(item) => item.into(),
            Err(err) => return Err(err.into()),
        };

        match item.into() {
            ReplyItem::Message(resp) => return Ok(Async::Ready(resp)),
            ReplyItem::Future(fut) => self.fut4 = Some(fut),
        }

        self.poll()
    }
}
