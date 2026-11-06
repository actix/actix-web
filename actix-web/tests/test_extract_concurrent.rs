use std::{
    cell::Cell,
    future::{poll_fn, Future},
    pin::pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

use actix_web::{
    dev::Payload, error::ErrorBadRequest, test::TestRequest, Error, FromRequest, HttpRequest,
};

#[derive(Default)]
struct State {
    first_polls: Cell<usize>,
    second_polls: Cell<usize>,
    release: Cell<bool>,
}

struct First;
struct Second;
struct Failure;

impl FromRequest for First {
    type Error = Error;

    fn from_request(
        req: &HttpRequest,
        _: &mut Payload,
    ) -> impl Future<Output = Result<Self, Error>> + use<> {
        let state = req.app_data::<Rc<State>>().unwrap().clone();

        poll_fn(move |_| {
            state.first_polls.set(state.first_polls.get() + 1);

            if state.release.get() {
                Poll::Ready(Ok(First))
            } else {
                Poll::Pending
            }
        })
    }
}

impl FromRequest for Second {
    type Error = Error;

    fn from_request(
        req: &HttpRequest,
        _: &mut Payload,
    ) -> impl Future<Output = Result<Self, Error>> + use<> {
        let state = req.app_data::<Rc<State>>().unwrap().clone();

        async move {
            state.second_polls.set(state.second_polls.get() + 1);
            Ok(Second)
        }
    }
}

impl FromRequest for Failure {
    type Error = Error;

    fn from_request(
        _: &HttpRequest,
        _: &mut Payload,
    ) -> impl Future<Output = Result<Self, Error>> + use<> {
        async { Err(ErrorBadRequest("extraction failed")) }
    }
}

#[test]
fn tuple_polls_later_extractors_while_first_is_pending() {
    let state = Rc::new(State::default());
    let req = TestRequest::default()
        .app_data(state.clone())
        .to_http_request();
    let mut payload = Payload::None;
    let future = <(First, Second)>::from_request(&req, &mut payload);

    // The returned future must not retain either input borrow.
    drop(req);
    drop(payload);

    let mut future = pin!(future);
    let mut cx = Context::from_waker(Waker::noop());

    assert!(future.as_mut().poll(&mut cx).is_pending());
    assert_eq!(state.first_polls.get(), 1);
    assert_eq!(state.second_polls.get(), 1);

    state.release.set(true);

    assert!(future.as_mut().poll(&mut cx).is_ready());
    assert_eq!(state.first_polls.get(), 2);
    assert_eq!(state.second_polls.get(), 1);
}

#[test]
fn tuple_returns_later_error_while_first_is_pending() {
    let state = Rc::new(State::default());
    let req = TestRequest::default()
        .app_data(state.clone())
        .to_http_request();
    let mut payload = Payload::None;
    let mut future = pin!(<(First, Failure, Second)>::from_request(&req, &mut payload));
    let mut cx = Context::from_waker(Waker::noop());

    let Poll::Ready(Err(err)) = future.as_mut().poll(&mut cx) else {
        panic!("tuple must return the error without waiting for the first extractor");
    };

    assert_eq!(err.to_string(), "extraction failed");
    assert_eq!(state.first_polls.get(), 1);
    assert_eq!(state.second_polls.get(), 0);
}
