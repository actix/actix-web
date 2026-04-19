use std::time::UNIX_EPOCH;

use actix_files::NamedFile;
use actix_web::{
    http::{header, StatusCode},
    test, web, App,
};
use filetime::{set_file_mtime, FileTime};
use tempfile::tempdir;

#[actix_web::test]
async fn serves_file_with_pre_epoch_mtime() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("pre_epoch.txt");

    std::fs::write(&path, b"hello").unwrap();

    // set mtime to before UNIX epoch; this used to panic during ETag/Last-Modified generation
    set_file_mtime(&path, FileTime::from_unix_time(-60, 0)).unwrap();

    let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
    assert!(
        mtime < UNIX_EPOCH,
        "fixture mtime should be before UNIX_EPOCH"
    );

    let srv = {
        let path = path.clone();
        test::init_service(App::new().default_service(web::to(move || {
            let path = path.clone();
            async move { NamedFile::open_async(path).await.unwrap() }
        })))
        .await
    };

    let req = test::TestRequest::with_uri("/").to_request();
    let res = test::call_service(&srv, req).await;

    assert_eq!(res.status(), StatusCode::OK);

    // ETag is still generated even for pre-epoch times.
    assert!(res.headers().contains_key(header::ETAG));

    // HTTP-date formatting in the httpdate crate does not support pre-epoch times.
    assert!(!res.headers().contains_key(header::LAST_MODIFIED));

    let body = test::read_body(res).await;
    assert_eq!(&body[..], b"hello");
}
