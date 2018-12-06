use std::marker::PhantomData;

use futures::{Async, Future, IntoFuture, Poll};

use super::{IntoNewService, IntoService, NewService, Service};

/// `Apply` service combinator
pub struct Apply<T, F, In, Out, Request>
where
    T: Service<Request>,
{
    service: T,
    f: F,
    r: PhantomData<(In, Out, Request)>,
}

impl<T, F, In, Out, Request> Apply<T, F, In, Out, Request>
where
    T: Service<Request>,
    F: FnMut(In, &mut T) -> Out,
    Out: IntoFuture,
{
    /// Create new `Apply` combinator
    pub fn new<I: IntoService<T, Request>>(service: I, f: F) -> Self {
        Self {
            service: service.into_service(),
            f,
            r: PhantomData,
        }
    }
}

impl<T, F, In, Out, Request> Clone for Apply<T, F, In, Out, Request>
where
    T: Service<Request> + Clone,
    F: Clone,
{
    fn clone(&self) -> Self {
        Apply {
            service: self.service.clone(),
            f: self.f.clone(),
            r: PhantomData,
        }
    }
}

impl<T, F, In, Out, Request> Service<In> for Apply<T, F, In, Out, Request>
where
    T: Service<Request, Error = Out::Error>,
    F: FnMut(In, &mut T) -> Out,
    Out: IntoFuture,
{
    type Response = <Out::Future as Future>::Item;
    type Error = <Out::Future as Future>::Error;
    type Future = Out::Future;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready().map_err(|e| e.into())
    }

    fn call(&mut self, req: In) -> Self::Future {
        (self.f)(req, &mut self.service).into_future()
    }
}

/// `ApplyNewService` new service combinator
pub struct ApplyNewService<T, F, In, Out, Request>
where
    T: NewService<Request>,
{
    service: T,
    f: F,
    r: PhantomData<(In, Out, Request)>,
}

impl<T, F, In, Out, Request> ApplyNewService<T, F, In, Out, Request>
where
    T: NewService<Request>,
    F: FnMut(In, &mut T::Service) -> Out,
    Out: IntoFuture,
{
    /// Create new `ApplyNewService` new service instance
    pub fn new<F1: IntoNewService<T, Request>>(service: F1, f: F) -> Self {
        Self {
            f,
            service: service.into_new_service(),
            r: PhantomData,
        }
    }
}

impl<T, F, In, Out, Request> Clone for ApplyNewService<T, F, In, Out, Request>
where
    T: NewService<Request> + Clone,
    F: FnMut(Out, &mut T::Service) -> Out + Clone,
    Out: IntoFuture,
{
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            f: self.f.clone(),
            r: PhantomData,
        }
    }
}

impl<T, F, In, Out, Request> NewService<In> for ApplyNewService<T, F, In, Out, Request>
where
    T: NewService<Request, Error = Out::Error>,
    F: FnMut(In, &mut T::Service) -> Out + Clone,
    Out: IntoFuture,
{
    type Response = <Out::Future as Future>::Item;
    type Error = <Out::Future as Future>::Error;
    type Service = Apply<T::Service, F, In, Out, Request>;

    type InitError = T::InitError;
    type Future = ApplyNewServiceFuture<T, F, In, Out, Request>;

    fn new_service(&self) -> Self::Future {
        ApplyNewServiceFuture::new(self.service.new_service(), self.f.clone())
    }
}

pub struct ApplyNewServiceFuture<T, F, In, Out, Request>
where
    T: NewService<Request>,
    F: FnMut(In, &mut T::Service) -> Out,
    Out: IntoFuture,
{
    fut: T::Future,
    f: Option<F>,
    r: PhantomData<(In, Out)>,
}

impl<T, F, In, Out, Request> ApplyNewServiceFuture<T, F, In, Out, Request>
where
    T: NewService<Request>,
    F: FnMut(In, &mut T::Service) -> Out,
    Out: IntoFuture,
{
    fn new(fut: T::Future, f: F) -> Self {
        ApplyNewServiceFuture {
            f: Some(f),
            fut,
            r: PhantomData,
        }
    }
}

impl<T, F, In, Out, Request> Future for ApplyNewServiceFuture<T, F, In, Out, Request>
where
    T: NewService<Request>,
    F: FnMut(In, &mut T::Service) -> Out,
    Out: IntoFuture,
{
    type Item = Apply<T::Service, F, In, Out, Request>;
    type Error = T::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Async::Ready(service) = self.fut.poll()? {
            Ok(Async::Ready(Apply::new(service, self.f.take().unwrap())))
        } else {
            Ok(Async::NotReady)
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::future::{ok, FutureResult};
    use futures::{Async, Future, Poll};

    use service::{
        IntoNewService, IntoService, NewService, NewServiceExt, Service, ServiceExt,
    };

    #[derive(Clone)]
    struct Srv;
    impl Service<()> for Srv {
        type Response = ();
        type Error = ();
        type Future = FutureResult<(), ()>;

        fn poll_ready(&mut self) -> Poll<(), Self::Error> {
            Ok(Async::Ready(()))
        }

        fn call(&mut self, _: ()) -> Self::Future {
            ok(())
        }
    }

    #[test]
    fn test_call() {
        let blank = |req| Ok(req);

        let mut srv = blank.into_service().apply(Srv, |req: &'static str, srv| {
            srv.call(()).map(move |res| (req, res))
        });
        assert!(srv.poll_ready().is_ok());
        let res = srv.call("srv").poll();
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), Async::Ready(("srv", ())));
    }

    #[test]
    fn test_new_service() {
        let blank = || Ok::<_, ()>((|req| Ok(req)).into_service());

        let new_srv = blank.into_new_service().apply(
            || Ok(Srv),
            |req: &'static str, srv| srv.call(()).map(move |res| (req, res)),
        );
        if let Async::Ready(mut srv) = new_srv.new_service().poll().unwrap() {
            assert!(srv.poll_ready().is_ok());
            let res = srv.call("srv").poll();
            assert!(res.is_ok());
            assert_eq!(res.unwrap(), Async::Ready(("srv", ())));
        } else {
            panic!()
        }
    }
}
