use std::io::Write;

use actix_files::NamedFile;
use actix_web::{
    body::to_bytes,
    http::{header, StatusCode},
    test::TestRequest,
};

#[actix_web::test]
async fn byte_range_units_are_case_insensitive() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(b"0123456789").unwrap();

    for range in ["Bytes=2-4", "BYTES=2-4", "bYtEs=2-4", "bytes=2-4"] {
        let req = TestRequest::get()
            .insert_header((header::RANGE, range))
            .to_http_request();
        let response = NamedFile::open(file.path()).unwrap().into_response(&req);

        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT, "{range}");
        assert_eq!(
            response.headers().get(header::CONTENT_RANGE).unwrap(),
            "bytes 2-4/10"
        );
        assert_eq!(to_bytes(response.into_body()).await.unwrap(), b"234"[..]);
    }
}

#[actix_web::test]
async fn unknown_range_units_are_ignored() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(b"0123456789").unwrap();

    for range in ["items=2-4", "BYTESX=2-4", "items=first", "items=100-200"] {
        let req = TestRequest::get()
            .insert_header((header::RANGE, range))
            .to_http_request();
        let response = NamedFile::open(file.path()).unwrap().into_response(&req);

        assert_eq!(response.status(), StatusCode::OK, "{range}");
        assert!(!response.headers().contains_key(header::CONTENT_RANGE));
        assert_eq!(
            to_bytes(response.into_body()).await.unwrap(),
            b"0123456789"[..]
        );
    }
}

#[actix_web::test]
async fn invalid_byte_ranges_remain_unsatisfiable() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(b"0123456789").unwrap();

    for range in [
        "bytes=4-2",
        "Bytes=4-2",
        "bytes=10-",
        "BYTES=10-",
        "bytes=",
        "Bytes=",
    ] {
        let req = TestRequest::get()
            .insert_header((header::RANGE, range))
            .to_http_request();
        let response = NamedFile::open(file.path()).unwrap().into_response(&req);

        assert_eq!(
            response.status(),
            StatusCode::RANGE_NOT_SATISFIABLE,
            "{range}"
        );
        assert_eq!(
            response.headers().get(header::CONTENT_RANGE).unwrap(),
            "bytes */10"
        );
        assert!(to_bytes(response.into_body()).await.unwrap().is_empty());
    }
}
