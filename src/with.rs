use futures::{Async, Future, Poll};
use std::marker::PhantomData;
use std::rc::Rc;

use error::Error;
use handler::{AsyncResult, AsyncResultItem, FromRequest, Handler, Responder};
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

trait FnWith<T, R>: 'static {
    fn call_with(self: &Self, T) -> R;
}

impl<T, R, F: Fn(T) -> R + 'static> FnWith<T, R> for F {
    #[cfg_attr(feature = "cargo-clippy", allow(boxed_local))]
    fn call_with(self: &Self, arg: T) -> R {
        (*self)(arg)
    }
}

#[doc(hidden)]
pub trait WithFactory<T, S, R>: 'static
where
    T: FromRequest<S>,
    R: Responder,
{
    fn create(self) -> With<T, S, R>;

    fn create_with_config(self, T::Config) -> With<T, S, R>;
}

#[doc(hidden)]
pub trait WithAsyncFactory<T, S, R, I, E>: 'static
where
    T: FromRequest<S>,
    R: Future<Item = I, Error = E>,
    I: Responder,
    E: Into<Error>,
{
    fn create(self) -> WithAsync<T, S, R, I, E>;

    fn create_with_config(self, T::Config) -> WithAsync<T, S, R, I, E>;
}

// impl<T1, T2, T3, S, F, R> WithFactory<(T1, T2, T3), S, R> for F
// where F: Fn(T1, T2, T3) -> R + 'static,
//       T1: FromRequest<S> + 'static,
//       T2: FromRequest<S> + 'static,
//       T3: FromRequest<S> + 'static,
//       R: Responder + 'static,
//       S: 'static,
// {
//     fn create(self) -> With<(T1, T2, T3), S, R> {
//         With::new(move |(t1, t2, t3)| (self)(t1, t2, t3), (
//             T1::Config::default(), T2::Config::default(), T3::Config::default()))
//     }

//     fn create_with_config(self, cfg: (T1::Config, T2::Config, T3::Config,)) -> With<(T1, T2, T3), S, R> {
//         With::new(move |(t1, t2, t3)| (self)(t1, t2, t3), cfg)
//     }
// }

#[doc(hidden)]
pub struct With<T, S, R>
where
    T: FromRequest<S>,
    S: 'static,
{
    hnd: Rc<FnWith<T, R>>,
    cfg: Rc<T::Config>,
    _s: PhantomData<S>,
}

impl<T, S, R> With<T, S, R>
where
    T: FromRequest<S>,
    S: 'static,
{
    pub fn new<F: Fn(T) -> R + 'static>(f: F, cfg: T::Config) -> Self {
        With {
            cfg: Rc::new(cfg),
            hnd: Rc::new(f),
            _s: PhantomData,
        }
    }
}

impl<T, S, R> Handler<S> for With<T, S, R>
where
    R: Responder + 'static,
    T: FromRequest<S> + 'static,
    S: 'static,
{
    type Result = AsyncResult<HttpResponse>;

    fn handle(&self, req: &HttpRequest<S>) -> Self::Result {
        let mut fut = WithHandlerFut {
            req: req.clone(),
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

struct WithHandlerFut<T, S, R>
where
    R: Responder,
    T: FromRequest<S> + 'static,
    S: 'static,
{
    started: bool,
    hnd: Rc<FnWith<T, R>>,
    cfg: Rc<T::Config>,
    req: HttpRequest<S>,
    fut1: Option<Box<Future<Item = T, Error = Error>>>,
    fut2: Option<Box<Future<Item = HttpResponse, Error = Error>>>,
}

impl<T, S, R> Future for WithHandlerFut<T, S, R>
where
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

        let item = match self.hnd.as_ref().call_with(item).respond_to(&self.req) {
            Ok(item) => item.into(),
            Err(e) => return Err(e.into()),
        };

        match item.into() {
            AsyncResultItem::Err(err) => Err(err),
            AsyncResultItem::Ok(resp) => Ok(Async::Ready(resp)),
            AsyncResultItem::Future(fut) => {
                self.fut2 = Some(fut);
                self.poll()
            }
        }
    }
}

#[doc(hidden)]
pub struct WithAsync<T, S, R, I, E>
where
    R: Future<Item = I, Error = E>,
    I: Responder,
    E: Into<E>,
    T: FromRequest<S>,
    S: 'static,
{
    hnd: Rc<FnWith<T, R>>,
    cfg: Rc<T::Config>,
    _s: PhantomData<S>,
}

impl<T, S, R, I, E> WithAsync<T, S, R, I, E>
where
    R: Future<Item = I, Error = E>,
    I: Responder,
    E: Into<Error>,
    T: FromRequest<S>,
    S: 'static,
{
    pub fn new<F: Fn(T) -> R + 'static>(f: F, cfg: T::Config) -> Self {
        WithAsync {
            cfg: Rc::new(cfg),
            hnd: Rc::new(f),
            _s: PhantomData,
        }
    }
}

impl<T, S, R, I, E> Handler<S> for WithAsync<T, S, R, I, E>
where
    R: Future<Item = I, Error = E> + 'static,
    I: Responder + 'static,
    E: Into<Error> + 'static,
    T: FromRequest<S> + 'static,
    S: 'static,
{
    type Result = AsyncResult<HttpResponse>;

    fn handle(&self, req: &HttpRequest<S>) -> Self::Result {
        let mut fut = WithAsyncHandlerFut {
            req: req.clone(),
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

struct WithAsyncHandlerFut<T, S, R, I, E>
where
    R: Future<Item = I, Error = E> + 'static,
    I: Responder + 'static,
    E: Into<Error> + 'static,
    T: FromRequest<S> + 'static,
    S: 'static,
{
    started: bool,
    hnd: Rc<FnWith<T, R>>,
    cfg: Rc<T::Config>,
    req: HttpRequest<S>,
    fut1: Option<Box<Future<Item = T, Error = Error>>>,
    fut2: Option<R>,
    fut3: Option<Box<Future<Item = HttpResponse, Error = Error>>>,
}

impl<T, S, R, I, E> Future for WithAsyncHandlerFut<T, S, R, I, E>
where
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

        self.fut2 = Some(self.hnd.as_ref().call_with(item));
        self.poll()
    }
}

macro_rules! with_factory_tuple ({$(($n:tt, $T:ident)),+} => {
    impl<$($T,)+ State, Func, Res> WithFactory<($($T,)+), State, Res> for Func
    where Func: Fn($($T,)+) -> Res + 'static,
        $($T: FromRequest<State> + 'static,)+
          Res: Responder + 'static,
          State: 'static,
    {
        fn create(self) -> With<($($T,)+), State, Res> {
            With::new(move |($($n,)+)| (self)($($n,)+), ($($T::Config::default(),)+))
        }

        fn create_with_config(self, cfg: ($($T::Config,)+)) -> With<($($T,)+), State, Res> {
            With::new(move |($($n,)+)| (self)($($n,)+), cfg)
        }
    }
});

macro_rules! with_async_factory_tuple ({$(($n:tt, $T:ident)),+} => {
    impl<$($T,)+ State, Func, Res, Item, Err> WithAsyncFactory<($($T,)+), State, Res, Item, Err> for Func
    where Func: Fn($($T,)+) -> Res + 'static,
        $($T: FromRequest<State> + 'static,)+
          Res: Future<Item=Item, Error=Err>,
          Item: Responder + 'static,
          Err: Into<Error>,
          State: 'static,
    {
        fn create(self) -> WithAsync<($($T,)+), State, Res, Item, Err> {
            WithAsync::new(move |($($n,)+)| (self)($($n,)+), ($($T::Config::default(),)+))
        }

        fn create_with_config(self, cfg: ($($T::Config,)+)) -> WithAsync<($($T,)+), State, Res, Item, Err> {
            WithAsync::new(move |($($n,)+)| (self)($($n,)+), cfg)
        }
    }
});

with_factory_tuple!((a, A));
with_factory_tuple!((a, A), (b, B));
with_factory_tuple!((a, A), (b, B), (c, C));
with_factory_tuple!((a, A), (b, B), (c, C), (d, D));
with_factory_tuple!((a, A), (b, B), (c, C), (d, D), (e, E));
with_factory_tuple!((a, A), (b, B), (c, C), (d, D), (e, E), (f, F));
with_factory_tuple!((a, A), (b, B), (c, C), (d, D), (e, E), (f, F), (g, G));
with_factory_tuple!(
    (a, A),
    (b, B),
    (c, C),
    (d, D),
    (e, E),
    (f, F),
    (g, G),
    (h, H)
);
with_factory_tuple!(
    (a, A),
    (b, B),
    (c, C),
    (d, D),
    (e, E),
    (f, F),
    (g, G),
    (h, H),
    (i, I)
);

with_async_factory_tuple!((a, A));
with_async_factory_tuple!((a, A), (b, B));
with_async_factory_tuple!((a, A), (b, B), (c, C));
with_async_factory_tuple!((a, A), (b, B), (c, C), (d, D));
with_async_factory_tuple!((a, A), (b, B), (c, C), (d, D), (e, E));
with_async_factory_tuple!((a, A), (b, B), (c, C), (d, D), (e, E), (f, F));
with_async_factory_tuple!((a, A), (b, B), (c, C), (d, D), (e, E), (f, F), (g, G));
with_async_factory_tuple!(
    (a, A),
    (b, B),
    (c, C),
    (d, D),
    (e, E),
    (f, F),
    (g, G),
    (h, H)
);
with_async_factory_tuple!(
    (a, A),
    (b, B),
    (c, C),
    (d, D),
    (e, E),
    (f, F),
    (g, G),
    (h, H),
    (i, I)
);
