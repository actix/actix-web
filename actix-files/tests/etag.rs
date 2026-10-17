#![cfg(any(unix, windows))]

use std::fs::{self, File};

use actix_files::NamedFile;
use actix_web::{
    body::to_bytes,
    http::{header, StatusCode},
    test::TestRequest,
};
use filetime::{set_file_mtime, FileTime};

#[actix_web::test]
async fn if_none_match_does_not_match_a_different_file_with_same_size_and_mtime() {
    let dir = tempfile::tempdir().unwrap();
    let first_path = dir.path().join("first.txt");
    let second_path = dir.path().join("second.txt");

    fs::write(&first_path, b"first").unwrap();
    fs::write(&second_path, b"other").unwrap();

    let mtime = FileTime::from_unix_time(1_600_000_000, 0);

    set_file_mtime(&first_path, mtime).unwrap();
    set_file_mtime(&second_path, mtime).unwrap();

    let first_md = fs::metadata(&first_path).unwrap();
    let second_md = fs::metadata(&second_path).unwrap();

    assert_eq!(first_md.len(), second_md.len());
    assert_eq!(first_md.modified().unwrap(), second_md.modified().unwrap());

    let req = TestRequest::default().to_http_request();
    let first = NamedFile::from_file(File::open(&first_path).unwrap(), "file.txt").unwrap();
    let res = first.into_response(&req);
    let etag = res.headers().get(header::ETAG).unwrap().clone();

    let req = TestRequest::default()
        .insert_header((header::IF_NONE_MATCH, etag.clone()))
        .to_http_request();
    let second = NamedFile::from_file(File::open(&second_path).unwrap(), "file.txt").unwrap();
    let res = second.into_response(&req);

    assert_eq!(res.status(), StatusCode::OK);
    assert_ne!(res.headers().get(header::ETAG).unwrap(), &etag);
    assert_eq!(
        to_bytes(res.into_body()).await.unwrap(),
        b"other".as_slice()
    );
}
