use actix_service::NewService;
use bytes::{Bytes, BytesMut};
use futures::future::{self, ok};
use futures::{Future, Stream};

use actix_http::{
    client, error::PayloadError, HttpMessage, HttpService, Request, Response,
};
use actix_http_test::TestServer;

const STR: &str = "Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World";

fn load_body<S>(stream: S) -> impl Future<Item = BytesMut, Error = PayloadError>
where
    S: Stream<Item = Bytes, Error = PayloadError>,
{
    stream.fold(BytesMut::new(), move |mut body, chunk| {
        body.extend_from_slice(&chunk);
        Ok::<_, PayloadError>(body)
    })
}

#[test]
fn test_h1_v2() {
    env_logger::init();
    let mut srv = TestServer::new(move || {
        HttpService::build()
            .finish(|_| future::ok::<_, ()>(Response::Ok().body(STR)))
            .map(|_| ())
    });
    let mut connector = srv.connector();

    let request = srv.get().finish().unwrap();
    let response = srv.block_on(request.send(&mut connector)).unwrap();
    assert!(response.status().is_success());

    let request = srv.get().header("x-test", "111").finish().unwrap();
    let repr = format!("{:?}", request);
    assert!(repr.contains("ClientRequest"));
    assert!(repr.contains("x-test"));

    let mut response = srv.block_on(request.send(&mut connector)).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.block_on(load_body(response.take_payload())).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    let request = srv.post().finish().unwrap();
    let mut response = srv.block_on(request.send(&mut connector)).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.block_on(load_body(response.take_payload())).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_connection_close() {
    let mut srv = TestServer::new(move || {
        HttpService::build()
            .finish(|_| ok::<_, ()>(Response::Ok().body(STR)))
            .map(|_| ())
    });
    let mut connector = srv.connector();

    let request = srv.get().close().finish().unwrap();
    let response = srv.block_on(request.send(&mut connector)).unwrap();
    assert!(response.status().is_success());
}

#[test]
fn test_with_query_parameter() {
    let mut srv = TestServer::new(move || {
        HttpService::build()
            .finish(|req: Request| {
                if req.uri().query().unwrap().contains("qp=") {
                    ok::<_, ()>(Response::Ok().finish())
                } else {
                    ok::<_, ()>(Response::BadRequest().finish())
                }
            })
            .map(|_| ())
    });
    let mut connector = srv.connector();

    let request = client::ClientRequest::get(srv.url("/?qp=5"))
        .finish()
        .unwrap();

    let response = srv.block_on(request.send(&mut connector)).unwrap();
    assert!(response.status().is_success());
}
