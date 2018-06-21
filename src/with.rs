use futures::{Async, Future, Poll};
use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::rc::Rc;

use error::Error;
use handler::{AsyncResult, AsyncResultItem, FromRequest, Handler, Responder};
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

pub(crate) struct With<T, S, F, R>
where
    F: Fn(T) -> R,
    T: FromRequest<S>,
    S: 'static,
{
    hnd: Rc<WithHnd<T, S, F, R>>,
    cfg: Rc<T::Config>,
}

pub struct WithHnd<T, S, F, R>
where
    F: Fn(T) -> R,
    T: FromRequest<S>,
    S: 'static,
{
    hnd: Rc<UnsafeCell<F>>,
    _t: PhantomData<T>,
    _s: PhantomData<S>,
}

impl<T, S, F, R> With<T, S, F, R>
where
    F: Fn(T) -> R,
    T: FromRequest<S>,
    S: 'static,
{
    pub fn new(f: F, cfg: T::Config) -> Self {
        With {
            cfg: Rc::new(cfg),
            hnd: Rc::new(WithHnd {
                hnd: Rc::new(UnsafeCell::new(f)),
                _t: PhantomData,
                _s: PhantomData,
            }),
        }
    }
}

impl<T, S, F, R> Handler<S> for With<T, S, F, R>
where
    F: Fn(T) -> R + 'static,
    R: Responder + 'static,
    T: FromRequest<S> + 'static,
    S: 'static,
{
    type Result = AsyncResult<HttpResponse>;

    fn handle(&self, req: HttpRequest<S>) -> Self::Result {
        let mut fut = WithHandlerFut {
            req,
            started: false,
            hnd: Rc::clone(&self.hnd),
            cfg: self.cfg.clone(),
            fut1: None,
            fut2: None,
        };

        match fut.poll() {
            Ok(Async::Ready(resp)) => AsyncResult::ok(resp),
            Ok(Async::NotReady) => AsyncResult::async(Box::new(fut)),
            Err(e) => AsyncResult::err(e),
        }
    }
}

struct WithHandlerFut<T, S, F, R>
where
    F: Fn(T) -> R,
    R: Responder,
    T: FromRequest<S> + 'static,
    S: 'static,
{
    started: bool,
    hnd: Rc<WithHnd<T, S, F, R>>,
    cfg: Rc<T::Config>,
    req: HttpRequest<S>,
    fut1: Option<Box<Future<Item = T, Error = Error>>>,
    fut2: Option<Box<Future<Item = HttpResponse, Error = Error>>>,
}

impl<T, S, F, R> Future for WithHandlerFut<T, S, F, R>
where
    F: Fn(T) -> R,
    R: Responder + 'static,
    T: FromRequest<S> + 'static,
    S: 'static,
{
    type Item = HttpResponse;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut2 {
            return fut.poll();
        }

        let item = if !self.started {
            self.started = true;
            let reply = T::from_request(&self.req, self.cfg.as_ref()).into();
            match reply.into() {
                AsyncResultItem::Err(err) => return Err(err),
                AsyncResultItem::Ok(msg) => msg,
                AsyncResultItem::Future(fut) => {
                    self.fut1 = Some(fut);
                    return self.poll();
                }
            }
        } else {
            match self.fut1.as_mut().unwrap().poll()? {
                Async::Ready(item) => item,
                Async::NotReady => return Ok(Async::NotReady),
            }
        };

        let fut = {
            // clone handler, inicrease ref counter
            let h = self.hnd.as_ref().hnd.clone();
            // Enforce invariants before entering unsafe code.
            // Only two references could exists With struct owns one, and line above
            if Rc::weak_count(&h) != 0 || Rc::strong_count(&h) != 2 {
                panic!("Multiple copies of handler are in use")
            }
            let hnd: &mut F = unsafe { &mut *h.as_ref().get() };
            let item = match (*hnd)(item).respond_to(&self.req) {
                Ok(item) => item.into(),
                Err(e) => return Err(e.into()),
            };

            match item.into() {
                AsyncResultItem::Err(err) => return Err(err),
                AsyncResultItem::Ok(resp) => return Ok(Async::Ready(resp)),
                AsyncResultItem::Future(fut) => fut,
            }
        };
        self.fut2 = Some(fut);
        self.poll()
    }
}

pub(crate) struct WithAsync<T, S, F, R, I, E>
where
    F: Fn(T) -> R,
    R: Future<Item = I, Error = E>,
    I: Responder,
    E: Into<E>,
    T: FromRequest<S>,
    S: 'static,
{
    hnd: Rc<WithHnd<T, S, F, R>>,
    cfg: Rc<T::Config>,
}

impl<T, S, F, R, I, E> WithAsync<T, S, F, R, I, E>
where
    F: Fn(T) -> R,
    R: Future<Item = I, Error = E>,
    I: Responder,
    E: Into<Error>,
    T: FromRequest<S>,
    S: 'static,
{
    pub fn new(f: F, cfg: T::Config) -> Self {
        WithAsync {
            cfg: Rc::new(cfg),
            hnd: Rc::new(WithHnd {
                hnd: Rc::new(UnsafeCell::new(f)),
                _s: PhantomData,
                _t: PhantomData,
            }),
        }
    }
}

impl<T, S, F, R, I, E> Handler<S> for WithAsync<T, S, F, R, I, E>
where
    F: Fn(T) -> R + 'static,
    R: Future<Item = I, Error = E> + 'static,
    I: Responder + 'static,
    E: Into<Error> + 'static,
    T: FromRequest<S> + 'static,
    S: 'static,
{
    type Result = AsyncResult<HttpResponse>;

    fn handle(&self, req: HttpRequest<S>) -> Self::Result {
        let mut fut = WithAsyncHandlerFut {
            req,
            started: false,
            hnd: Rc::clone(&self.hnd),
            cfg: Rc::clone(&self.cfg),
            fut1: None,
            fut2: None,
            fut3: None,
        };

        match fut.poll() {
            Ok(Async::Ready(resp)) => AsyncResult::ok(resp),
            Ok(Async::NotReady) => AsyncResult::async(Box::new(fut)),
            Err(e) => AsyncResult::err(e),
        }
    }
}

struct WithAsyncHandlerFut<T, S, F, R, I, E>
where
    F: Fn(T) -> R,
    R: Future<Item = I, Error = E> + 'static,
    I: Responder + 'static,
    E: Into<Error> + 'static,
    T: FromRequest<S> + 'static,
    S: 'static,
{
    started: bool,
    hnd: Rc<WithHnd<T, S, F, R>>,
    cfg: Rc<T::Config>,
    req: HttpRequest<S>,
    fut1: Option<Box<Future<Item = T, Error = Error>>>,
    fut2: Option<R>,
    fut3: Option<Box<Future<Item = HttpResponse, Error = Error>>>,
}

impl<T, S, F, R, I, E> Future for WithAsyncHandlerFut<T, S, F, R, I, E>
where
    F: Fn(T) -> R,
    R: Future<Item = I, Error = E> + 'static,
    I: Responder + 'static,
    E: Into<Error> + 'static,
    T: FromRequest<S> + 'static,
    S: 'static,
{
    type Item = HttpResponse;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut3 {
            return fut.poll();
        }

        if self.fut2.is_some() {
            return match self.fut2.as_mut().unwrap().poll() {
                Ok(Async::NotReady) => Ok(Async::NotReady),
                Ok(Async::Ready(r)) => match r.respond_to(&self.req) {
                    Ok(r) => match r.into().into() {
                        AsyncResultItem::Err(err) => Err(err),
                        AsyncResultItem::Ok(resp) => Ok(Async::Ready(resp)),
                        AsyncResultItem::Future(fut) => {
                            self.fut3 = Some(fut);
                            self.poll()
                        }
                    },
                    Err(e) => Err(e.into()),
                },
                Err(e) => Err(e.into()),
            };
        }

        let item = if !self.started {
            self.started = true;
            let reply = T::from_request(&self.req, self.cfg.as_ref()).into();
            match reply.into() {
                AsyncResultItem::Err(err) => return Err(err),
                AsyncResultItem::Ok(msg) => msg,
                AsyncResultItem::Future(fut) => {
                    self.fut1 = Some(fut);
                    return self.poll();
                }
            }
        } else {
            match self.fut1.as_mut().unwrap().poll()? {
                Async::Ready(item) => item,
                Async::NotReady => return Ok(Async::NotReady),
            }
        };

        self.fut2 = {
            // clone handler, inicrease ref counter
            let h = self.hnd.as_ref().hnd.clone();
            // Enforce invariants before entering unsafe code.
            // Only two references could exists With struct owns one, and line above
            if Rc::weak_count(&h) != 0 || Rc::strong_count(&h) != 2 {
                panic!("Multiple copies of handler are in use")
            }
            let hnd: &mut F = unsafe { &mut *h.as_ref().get() };
            Some((*hnd)(item))
        };
        self.poll()
    }
}
