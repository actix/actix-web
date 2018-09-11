use std::marker;

use futures::{Future, Poll};

use super::NewService;

/// `MapInitErr` service combinator
pub struct MapInitErr<A, F, E> {
    a: A,
    f: F,
    e: marker::PhantomData<E>,
}

impl<A, F, E> MapInitErr<A, F, E>
where
    A: NewService,
    F: Fn(A::InitError) -> E,
{
    /// Create new `MapInitErr` combinator
    pub fn new(a: A, f: F) -> Self {
        Self {
            a,
            f,
            e: marker::PhantomData,
        }
    }
}

impl<A, F, E> Clone for MapInitErr<A, F, E>
where
    A: NewService + Clone,
    F: Fn(A::InitError) -> E + Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            f: self.f.clone(),
            e: marker::PhantomData,
        }
    }
}

impl<A, F, E> NewService for MapInitErr<A, F, E>
where
    A: NewService,
    F: Fn(A::InitError) -> E + Clone,
{
    type Request = A::Request;
    type Response = A::Response;
    type Error = A::Error;
    type Service = A::Service;

    type InitError = E;
    type Future = MapInitErrFuture<A, F, E>;

    fn new_service(&self) -> Self::Future {
        MapInitErrFuture::new(self.a.new_service(), self.f.clone())
    }
}

pub struct MapInitErrFuture<A, F, E>
where
    A: NewService,
    F: Fn(A::InitError) -> E,
{
    f: F,
    fut: A::Future,
}

impl<A, F, E> MapInitErrFuture<A, F, E>
where
    A: NewService,
    F: Fn(A::InitError) -> E,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapInitErrFuture { f, fut }
    }
}

impl<A, F, E> Future for MapInitErrFuture<A, F, E>
where
    A: NewService,
    F: Fn(A::InitError) -> E,
{
    type Item = A::Service;
    type Error = E;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.fut.poll().map_err(&self.f)
    }
}
