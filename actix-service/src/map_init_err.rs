use std::marker::PhantomData;

use futures::{Future, Poll};

use super::NewService;

/// `MapInitErr` service combinator
pub struct MapInitErr<A, F, E> {
    a: A,
    f: F,
    e: PhantomData<E>,
}

impl<A, F, E> MapInitErr<A, F, E> {
    /// Create new `MapInitErr` combinator
    pub fn new<Request>(a: A, f: F) -> Self
    where
        A: NewService<Request>,
        F: Fn(A::InitError) -> E,
    {
        Self {
            a,
            f,
            e: PhantomData,
        }
    }
}

impl<A, F, E> Clone for MapInitErr<A, F, E>
where
    A: Clone,
    F: Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            f: self.f.clone(),
            e: PhantomData,
        }
    }
}

impl<A, F, E, Request> NewService<Request> for MapInitErr<A, F, E>
where
    A: NewService<Request>,
    F: Fn(A::InitError) -> E + Clone,
{
    type Response = A::Response;
    type Error = A::Error;
    type Service = A::Service;

    type InitError = E;
    type Future = MapInitErrFuture<A, F, E, Request>;

    fn new_service(&self) -> Self::Future {
        MapInitErrFuture::new(self.a.new_service(), self.f.clone())
    }
}

pub struct MapInitErrFuture<A, F, E, Request>
where
    A: NewService<Request>,
    F: Fn(A::InitError) -> E,
{
    f: F,
    fut: A::Future,
}

impl<A, F, E, Request> MapInitErrFuture<A, F, E, Request>
where
    A: NewService<Request>,
    F: Fn(A::InitError) -> E,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapInitErrFuture { f, fut }
    }
}

impl<A, F, E, Request> Future for MapInitErrFuture<A, F, E, Request>
where
    A: NewService<Request>,
    F: Fn(A::InitError) -> E,
{
    type Item = A::Service;
    type Error = E;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.fut.poll().map_err(&self.f)
    }
}
