use std::{
    cell::Cell,
    future::{poll_fn, Future},
    pin::pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

use actix_web::{
    dev::Payload, error::ErrorBadRequest, test::TestRequest, Error, FromRequest, FromRequestBridge,
    HttpRequest,
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

impl FromRequestBridge for First {
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

impl FromRequestBridge for Second {
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

impl FromRequestBridge for Failure {
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

struct BodyLength(usize);

impl FromRequestBridge for BodyLength {
    type Error = Error;

    fn from_request(
        req: &HttpRequest,
        payload: &mut Payload,
    ) -> impl Future<Output = Result<Self, Self::Error>> + use<> {
        let body = actix_web::web::Bytes::from_request(req, payload);

        async { Ok(BodyLength(body.await?.len())) }
    }
}

struct Legacy<T>(T);

impl<T: Default> FromRequest for Legacy<T> {
    type Error = Error;
    type Future = std::future::Ready<Result<Self, Self::Error>>;

    fn from_request(_: &HttpRequest, _: &mut Payload) -> Self::Future {
        std::future::ready(Ok(Legacy(T::default())))
    }
}

#[actix_web::test]
async fn bridge_works_in_handlers_with_legacy_extractors() {
    use actix_web::{test, web, App};

    let app = test::init_service(App::new().route(
        "/",
        web::post().to(|body: BodyLength, legacy: Legacy<usize>| async move {
            (body.0 + legacy.0).to_string()
        }),
    ))
    .await;

    let request = TestRequest::post()
        .uri("/")
        .set_payload("hello")
        .to_request();
    let response = test::call_and_read_body(&app, request).await;

    assert_eq!(response, "5");
}

#[actix_web::test]
async fn bridge_works_with_option_result_and_extract() {
    let req = TestRequest::default().to_http_request();

    assert!(Option::<Failure>::extract(&req).await.unwrap().is_none());

    let error = Result::<Failure, Error>::extract(&req)
        .await
        .unwrap()
        .err()
        .unwrap();

    assert_eq!(error.to_string(), "extraction failed");
    assert!(<Failure as FromRequest>::extract(&req).await.is_err());
}
