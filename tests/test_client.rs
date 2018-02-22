extern crate actix;
extern crate actix_web;
extern crate bytes;
extern crate futures;

use bytes::Bytes;

use actix_web::*;


const STR: &str =
    "Hello World Hello World Hello World Hello World Hello World \
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


#[test]
fn test_simple() {
    let mut srv = test::TestServer::new(
        |app| app.handler(|_| httpcodes::HTTPOk.build().body(STR)));

    let request = srv.get().header("x-test", "111").finish().unwrap();
    let repr = format!("{:?}", request);
    assert!(repr.contains("ClientRequest"));
    assert!(repr.contains("x-test"));

    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    let request = srv.post().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}
