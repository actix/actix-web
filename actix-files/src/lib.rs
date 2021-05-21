//! Static file serving for Actix Web.
//!
//! Provides a non-blocking service for serving static files from disk.
//!
//! # Example
//! ```
//! use actix_web::App;
//! use actix_files::Files;
//!
//! let app = App::new()
//!     .service(Files::new("/static", ".").prefer_utf8(true));
//! ```

#![deny(rust_2018_idioms)]
#![warn(missing_docs, missing_debug_implementations)]

use actix_service::boxed::{BoxService, BoxServiceFactory};
use actix_web::{
    dev::{ServiceRequest, ServiceResponse},
    error::Error,
    http::header::DispositionType,
};
use mime_guess::from_ext;

mod chunked;
mod directory;
mod encoding;
mod error;
mod files;
mod named;
mod path_buf;
mod range;
mod service;

pub use crate::chunked::ChunkedReadFile;
pub use crate::directory::Directory;
pub use crate::files::Files;
pub use crate::named::NamedFile;
pub use crate::range::HttpRange;
pub use crate::service::FilesService;

use self::directory::{directory_listing, DirectoryRenderer};
use self::error::FilesError;
use self::path_buf::PathBufWrap;

type HttpService = BoxService<ServiceRequest, ServiceResponse, Error>;
type HttpNewService = BoxServiceFactory<(), ServiceRequest, ServiceResponse, Error, ()>;

/// Return the MIME type associated with a filename extension (case-insensitive).
/// If `ext` is empty or no associated type for the extension was found, returns
/// the type `application/octet-stream`.
#[inline]
pub fn file_extension_to_mime(ext: &str) -> mime::Mime {
    from_ext(ext).first_or_octet_stream()
}

type MimeOverride = dyn Fn(&mime::Name<'_>) -> DispositionType;

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, File},
        ops::Add,
        time::{Duration, SystemTime},
    };

    use actix_service::ServiceFactory;
    use actix_utils::future::ok;
    use actix_web::{
        guard,
        http::{
            header::{self, ContentDisposition, DispositionParam, DispositionType},
            Method, StatusCode,
        },
        middleware::Compress,
        test::{self, TestRequest},
        web::{self, Bytes},
        App, HttpResponse, Responder,
    };

    use super::*;

    #[actix_rt::test]
    async fn test_file_extension_to_mime() {
        let m = file_extension_to_mime("");
        assert_eq!(m, mime::APPLICATION_OCTET_STREAM);

        let m = file_extension_to_mime("jpg");
        assert_eq!(m, mime::IMAGE_JPEG);

        let m = file_extension_to_mime("invalid extension!!");
        assert_eq!(m, mime::APPLICATION_OCTET_STREAM);

        let m = file_extension_to_mime("");
        assert_eq!(m, mime::APPLICATION_OCTET_STREAM);
    }

    #[actix_rt::test]
    async fn test_if_modified_since_without_if_none_match() {
        let file = NamedFile::open("Cargo.toml").unwrap();
        let since = header::HttpDate::from(SystemTime::now().add(Duration::from_secs(60)));

        let req = TestRequest::default()
            .insert_header((header::IF_MODIFIED_SINCE, since))
            .to_http_request();
        let resp = file.respond_to(&req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
    }

    #[actix_rt::test]
    async fn test_if_modified_since_without_if_none_match_same() {
        let file = NamedFile::open("Cargo.toml").unwrap();
        let since = file.last_modified().unwrap();

        let req = TestRequest::default()
            .insert_header((header::IF_MODIFIED_SINCE, since))
            .to_http_request();
        let resp = file.respond_to(&req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
    }

    #[actix_rt::test]
    async fn test_if_modified_since_with_if_none_match() {
        let file = NamedFile::open("Cargo.toml").unwrap();
        let since = header::HttpDate::from(SystemTime::now().add(Duration::from_secs(60)));

        let req = TestRequest::default()
            .insert_header((header::IF_NONE_MATCH, "miss_etag"))
            .insert_header((header::IF_MODIFIED_SINCE, since))
            .to_http_request();
        let resp = file.respond_to(&req).await.unwrap();
        assert_ne!(resp.status(), StatusCode::NOT_MODIFIED);
    }

    #[actix_rt::test]
    async fn test_if_unmodified_since() {
        let file = NamedFile::open("Cargo.toml").unwrap();
        let since = file.last_modified().unwrap();

        let req = TestRequest::default()
            .insert_header((header::IF_UNMODIFIED_SINCE, since))
            .to_http_request();
        let resp = file.respond_to(&req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_if_unmodified_since_failed() {
        let file = NamedFile::open("Cargo.toml").unwrap();
        let since = header::HttpDate::from(SystemTime::UNIX_EPOCH);

        let req = TestRequest::default()
            .insert_header((header::IF_UNMODIFIED_SINCE, since))
            .to_http_request();
        let resp = file.respond_to(&req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::PRECONDITION_FAILED);
    }

    #[actix_rt::test]
    async fn test_named_file_text() {
        assert!(NamedFile::open("test--").is_err());
        let mut file = NamedFile::open("Cargo.toml").unwrap();
        {
            file.file();
            let _f: &File = &file;
        }
        {
            let _f: &mut File = &mut file;
        }

        let req = TestRequest::default().to_http_request();
        let resp = file.respond_to(&req).await.unwrap();
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/x-toml"
        );
        assert_eq!(
            resp.headers().get(header::CONTENT_DISPOSITION).unwrap(),
            "inline; filename=\"Cargo.toml\""
        );
    }

    #[actix_rt::test]
    async fn test_named_file_content_disposition() {
        assert!(NamedFile::open("test--").is_err());
        let mut file = NamedFile::open("Cargo.toml").unwrap();
        {
            file.file();
            let _f: &File = &file;
        }
        {
            let _f: &mut File = &mut file;
        }

        let req = TestRequest::default().to_http_request();
        let resp = file.respond_to(&req).await.unwrap();
        assert_eq!(
            resp.headers().get(header::CONTENT_DISPOSITION).unwrap(),
            "inline; filename=\"Cargo.toml\""
        );

        let file = NamedFile::open("Cargo.toml")
            .unwrap()
            .disable_content_disposition();
        let req = TestRequest::default().to_http_request();
        let resp = file.respond_to(&req).await.unwrap();
        assert!(resp.headers().get(header::CONTENT_DISPOSITION).is_none());
    }

    #[actix_rt::test]
    async fn test_named_file_non_ascii_file_name() {
        let mut file =
            NamedFile::from_file(File::open("Cargo.toml").unwrap(), "貨物.toml").unwrap();
        {
            file.file();
            let _f: &File = &file;
        }
        {
            let _f: &mut File = &mut file;
        }

        let req = TestRequest::default().to_http_request();
        let resp = file.respond_to(&req).await.unwrap();
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/x-toml"
        );
        assert_eq!(
            resp.headers().get(header::CONTENT_DISPOSITION).unwrap(),
            "inline; filename=\"貨物.toml\"; filename*=UTF-8''%E8%B2%A8%E7%89%A9.toml"
        );
    }

    #[actix_rt::test]
    async fn test_named_file_set_content_type() {
        let mut file = NamedFile::open("Cargo.toml")
            .unwrap()
            .set_content_type(mime::TEXT_XML);
        {
            file.file();
            let _f: &File = &file;
        }
        {
            let _f: &mut File = &mut file;
        }

        let req = TestRequest::default().to_http_request();
        let resp = file.respond_to(&req).await.unwrap();
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/xml"
        );
        assert_eq!(
            resp.headers().get(header::CONTENT_DISPOSITION).unwrap(),
            "inline; filename=\"Cargo.toml\""
        );
    }

    #[actix_rt::test]
    async fn test_named_file_image() {
        let mut file = NamedFile::open("tests/test.png").unwrap();
        {
            file.file();
            let _f: &File = &file;
        }
        {
            let _f: &mut File = &mut file;
        }

        let req = TestRequest::default().to_http_request();
        let resp = file.respond_to(&req).await.unwrap();
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/png"
        );
        assert_eq!(
            resp.headers().get(header::CONTENT_DISPOSITION).unwrap(),
            "inline; filename=\"test.png\""
        );
    }

    #[actix_rt::test]
    async fn test_named_file_image_attachment() {
        let cd = ContentDisposition {
            disposition: DispositionType::Attachment,
            parameters: vec![DispositionParam::Filename(String::from("test.png"))],
        };
        let mut file = NamedFile::open("tests/test.png")
            .unwrap()
            .set_content_disposition(cd);
        {
            file.file();
            let _f: &File = &file;
        }
        {
            let _f: &mut File = &mut file;
        }

        let req = TestRequest::default().to_http_request();
        let resp = file.respond_to(&req).await.unwrap();
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/png"
        );
        assert_eq!(
            resp.headers().get(header::CONTENT_DISPOSITION).unwrap(),
            "attachment; filename=\"test.png\""
        );
    }

    #[actix_rt::test]
    async fn test_named_file_binary() {
        let mut file = NamedFile::open("tests/test.binary").unwrap();
        {
            file.file();
            let _f: &File = &file;
        }
        {
            let _f: &mut File = &mut file;
        }

        let req = TestRequest::default().to_http_request();
        let resp = file.respond_to(&req).await.unwrap();
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/octet-stream"
        );
        assert_eq!(
            resp.headers().get(header::CONTENT_DISPOSITION).unwrap(),
            "attachment; filename=\"test.binary\""
        );
    }

    #[actix_rt::test]
    async fn test_named_file_status_code_text() {
        let mut file = NamedFile::open("Cargo.toml")
            .unwrap()
            .set_status_code(StatusCode::NOT_FOUND);
        {
            file.file();
            let _f: &File = &file;
        }
        {
            let _f: &mut File = &mut file;
        }

        let req = TestRequest::default().to_http_request();
        let resp = file.respond_to(&req).await.unwrap();
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/x-toml"
        );
        assert_eq!(
            resp.headers().get(header::CONTENT_DISPOSITION).unwrap(),
            "inline; filename=\"Cargo.toml\""
        );
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[actix_rt::test]
    async fn test_mime_override() {
        fn all_attachment(_: &mime::Name<'_>) -> DispositionType {
            DispositionType::Attachment
        }

        let srv = test::init_service(
            App::new().service(
                Files::new("/", ".")
                    .mime_override(all_attachment)
                    .index_file("Cargo.toml"),
            ),
        )
        .await;

        let request = TestRequest::get().uri("/").to_request();
        let response = test::call_service(&srv, request).await;
        assert_eq!(response.status(), StatusCode::OK);

        let content_disposition = response
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .expect("To have CONTENT_DISPOSITION");
        let content_disposition = content_disposition
            .to_str()
            .expect("Convert CONTENT_DISPOSITION to str");
        assert_eq!(content_disposition, "attachment; filename=\"Cargo.toml\"");
    }

    #[actix_rt::test]
    async fn test_named_file_ranges_status_code() {
        let srv = test::init_service(
            App::new().service(Files::new("/test", ".").index_file("Cargo.toml")),
        )
        .await;

        // Valid range header
        let request = TestRequest::get()
            .uri("/t%65st/Cargo.toml")
            .insert_header((header::RANGE, "bytes=10-20"))
            .to_request();
        let response = test::call_service(&srv, request).await;
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);

        // Invalid range header
        let request = TestRequest::get()
            .uri("/t%65st/Cargo.toml")
            .insert_header((header::RANGE, "bytes=1-0"))
            .to_request();
        let response = test::call_service(&srv, request).await;

        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    }

    #[actix_rt::test]
    async fn test_named_file_content_range_headers() {
        let srv = actix_test::start(|| App::new().service(Files::new("/", ".")));

        // Valid range header
        let response = srv
            .get("/tests/test.binary")
            .insert_header((header::RANGE, "bytes=10-20"))
            .send()
            .await
            .unwrap();
        let content_range = response.headers().get(header::CONTENT_RANGE).unwrap();
        assert_eq!(content_range.to_str().unwrap(), "bytes 10-20/100");

        // Invalid range header
        let response = srv
            .get("/tests/test.binary")
            .insert_header((header::RANGE, "bytes=10-5"))
            .send()
            .await
            .unwrap();
        let content_range = response.headers().get(header::CONTENT_RANGE).unwrap();
        assert_eq!(content_range.to_str().unwrap(), "bytes */100");
    }

    #[actix_rt::test]
    async fn test_named_file_content_length_headers() {
        let srv = actix_test::start(|| App::new().service(Files::new("/", ".")));

        // Valid range header
        let response = srv
            .get("/tests/test.binary")
            .insert_header((header::RANGE, "bytes=10-20"))
            .send()
            .await
            .unwrap();
        let content_length = response.headers().get(header::CONTENT_LENGTH).unwrap();
        assert_eq!(content_length.to_str().unwrap(), "11");

        // Valid range header, starting from 0
        let response = srv
            .get("/tests/test.binary")
            .insert_header((header::RANGE, "bytes=0-20"))
            .send()
            .await
            .unwrap();
        let content_length = response.headers().get(header::CONTENT_LENGTH).unwrap();
        assert_eq!(content_length.to_str().unwrap(), "21");

        // Without range header
        let mut response = srv.get("/tests/test.binary").send().await.unwrap();
        let content_length = response.headers().get(header::CONTENT_LENGTH).unwrap();
        assert_eq!(content_length.to_str().unwrap(), "100");

        // Should be no transfer-encoding
        let transfer_encoding = response.headers().get(header::TRANSFER_ENCODING);
        assert!(transfer_encoding.is_none());

        // Check file contents
        let bytes = response.body().await.unwrap();
        let data = web::Bytes::from(fs::read("tests/test.binary").unwrap());
        assert_eq!(bytes, data);
    }

    #[actix_rt::test]
    async fn test_head_content_length_headers() {
        let srv = actix_test::start(|| App::new().service(Files::new("/", ".")));

        let response = srv.head("/tests/test.binary").send().await.unwrap();

        let content_length = response
            .headers()
            .get(header::CONTENT_LENGTH)
            .unwrap()
            .to_str()
            .unwrap();

        assert_eq!(content_length, "100");
    }

    #[actix_rt::test]
    async fn test_static_files_with_spaces() {
        let srv = test::init_service(
            App::new().service(Files::new("/", ".").index_file("Cargo.toml")),
        )
        .await;
        let request = TestRequest::get()
            .uri("/tests/test%20space.binary")
            .to_request();
        let response = test::call_service(&srv, request).await;
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = test::read_body(response).await;
        let data = web::Bytes::from(fs::read("tests/test space.binary").unwrap());
        assert_eq!(bytes, data);
    }

    #[actix_rt::test]
    async fn test_files_not_allowed() {
        let srv = test::init_service(App::new().service(Files::new("/", "."))).await;

        let req = TestRequest::default()
            .uri("/Cargo.toml")
            .method(Method::POST)
            .to_request();

        let resp = test::call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);

        let srv = test::init_service(App::new().service(Files::new("/", "."))).await;
        let req = TestRequest::default()
            .method(Method::PUT)
            .uri("/Cargo.toml")
            .to_request();
        let resp = test::call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[actix_rt::test]
    async fn test_files_guards() {
        let srv = test::init_service(
            App::new().service(Files::new("/", ".").method_guard(guard::Post())),
        )
        .await;

        let req = TestRequest::default()
            .uri("/Cargo.toml")
            .method(Method::POST)
            .to_request();

        let resp = test::call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_named_file_content_encoding() {
        let srv = test::init_service(App::new().wrap(Compress::default()).service(
            web::resource("/").to(|| async {
                NamedFile::open("Cargo.toml")
                    .unwrap()
                    .set_content_encoding(header::ContentEncoding::Identity)
            }),
        ))
        .await;

        let request = TestRequest::get()
            .uri("/")
            .insert_header((header::ACCEPT_ENCODING, "gzip"))
            .to_request();
        let res = test::call_service(&srv, request).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert!(!res.headers().contains_key(header::CONTENT_ENCODING));
    }

    #[actix_rt::test]
    async fn test_named_file_content_encoding_gzip() {
        let srv = test::init_service(App::new().wrap(Compress::default()).service(
            web::resource("/").to(|| async {
                NamedFile::open("Cargo.toml")
                    .unwrap()
                    .set_content_encoding(header::ContentEncoding::Gzip)
            }),
        ))
        .await;

        let request = TestRequest::get()
            .uri("/")
            .insert_header((header::ACCEPT_ENCODING, "gzip"))
            .to_request();
        let res = test::call_service(&srv, request).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()
                .get(header::CONTENT_ENCODING)
                .unwrap()
                .to_str()
                .unwrap(),
            "gzip"
        );
    }

    #[actix_rt::test]
    async fn test_named_file_allowed_method() {
        let req = TestRequest::default().method(Method::GET).to_http_request();
        let file = NamedFile::open("Cargo.toml").unwrap();
        let resp = file.respond_to(&req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_static_files() {
        let srv =
            test::init_service(App::new().service(Files::new("/", ".").show_files_listing()))
                .await;
        let req = TestRequest::with_uri("/missing").to_request();

        let resp = test::call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let srv = test::init_service(App::new().service(Files::new("/", "."))).await;

        let req = TestRequest::default().to_request();
        let resp = test::call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let srv =
            test::init_service(App::new().service(Files::new("/", ".").show_files_listing()))
                .await;
        let req = TestRequest::with_uri("/tests").to_request();
        let resp = test::call_service(&srv, req).await;
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );

        let bytes = test::read_body(resp).await;
        assert!(format!("{:?}", bytes).contains("/tests/test.png"));
    }

    #[actix_rt::test]
    async fn test_redirect_to_slash_directory() {
        // should not redirect if no index and files listing is disabled
        let srv = test::init_service(
            App::new().service(Files::new("/", ".").redirect_to_slash_directory()),
        )
        .await;
        let req = TestRequest::with_uri("/tests").to_request();
        let resp = test::call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        // should redirect if index present
        let srv = test::init_service(
            App::new().service(
                Files::new("/", ".")
                    .index_file("test.png")
                    .redirect_to_slash_directory(),
            ),
        )
        .await;
        let req = TestRequest::with_uri("/tests").to_request();
        let resp = test::call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::FOUND);

        // should redirect if files listing is enabled
        let srv = test::init_service(
            App::new().service(
                Files::new("/", ".")
                    .show_files_listing()
                    .redirect_to_slash_directory(),
            ),
        )
        .await;
        let req = TestRequest::with_uri("/tests").to_request();
        let resp = test::call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::FOUND);

        // should not redirect if the path is wrong
        let req = TestRequest::with_uri("/not_existing").to_request();
        let resp = test::call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[actix_rt::test]
    async fn test_static_files_bad_directory() {
        let service = Files::new("/", "./missing").new_service(()).await.unwrap();

        let req = TestRequest::with_uri("/").to_srv_request();
        let resp = test::call_service(&service, req).await;

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[actix_rt::test]
    async fn test_default_handler_file_missing() {
        let st = Files::new("/", ".")
            .default_handler(|req: ServiceRequest| {
                ok(req.into_response(HttpResponse::Ok().body("default content")))
            })
            .new_service(())
            .await
            .unwrap();
        let req = TestRequest::with_uri("/missing").to_srv_request();
        let resp = test::call_service(&st, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = test::read_body(resp).await;
        assert_eq!(bytes, web::Bytes::from_static(b"default content"));
    }

    #[actix_rt::test]
    async fn test_serve_index_nested() {
        let service = Files::new(".", ".")
            .index_file("lib.rs")
            .new_service(())
            .await
            .unwrap();

        let req = TestRequest::default().uri("/src").to_srv_request();
        let resp = test::call_service(&service, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/x-rust"
        );
        assert_eq!(
            resp.headers().get(header::CONTENT_DISPOSITION).unwrap(),
            "inline; filename=\"lib.rs\""
        );
    }

    #[actix_rt::test]
    async fn integration_serve_index() {
        let srv = test::init_service(
            App::new().service(Files::new("test", ".").index_file("Cargo.toml")),
        )
        .await;

        let req = TestRequest::get().uri("/test").to_request();
        let res = test::call_service(&srv, req).await;
        assert_eq!(res.status(), StatusCode::OK);

        let bytes = test::read_body(res).await;

        let data = Bytes::from(fs::read("Cargo.toml").unwrap());
        assert_eq!(bytes, data);

        let req = TestRequest::get().uri("/test/").to_request();
        let res = test::call_service(&srv, req).await;
        assert_eq!(res.status(), StatusCode::OK);

        let bytes = test::read_body(res).await;
        let data = Bytes::from(fs::read("Cargo.toml").unwrap());
        assert_eq!(bytes, data);

        // nonexistent index file
        let req = TestRequest::get().uri("/test/unknown").to_request();
        let res = test::call_service(&srv, req).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);

        let req = TestRequest::get().uri("/test/unknown/").to_request();
        let res = test::call_service(&srv, req).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[actix_rt::test]
    async fn integration_percent_encoded() {
        let srv = test::init_service(
            App::new().service(Files::new("test", ".").index_file("Cargo.toml")),
        )
        .await;

        let req = TestRequest::get().uri("/test/%43argo.toml").to_request();
        let res = test::call_service(&srv, req).await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_serve_named_file() {
        let srv =
            test::init_service(App::new().service(NamedFile::open("Cargo.toml").unwrap()))
                .await;

        let req = TestRequest::get().uri("/Cargo.toml").to_request();
        let res = test::call_service(&srv, req).await;
        assert_eq!(res.status(), StatusCode::OK);

        let bytes = test::read_body(res).await;
        let data = Bytes::from(fs::read("Cargo.toml").unwrap());
        assert_eq!(bytes, data);

        let req = TestRequest::get().uri("/test/unknown").to_request();
        let res = test::call_service(&srv, req).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[actix_rt::test]
    async fn test_serve_named_file_prefix() {
        let srv = test::init_service(
            App::new()
                .service(web::scope("/test").service(NamedFile::open("Cargo.toml").unwrap())),
        )
        .await;

        let req = TestRequest::get().uri("/test/Cargo.toml").to_request();
        let res = test::call_service(&srv, req).await;
        assert_eq!(res.status(), StatusCode::OK);

        let bytes = test::read_body(res).await;
        let data = Bytes::from(fs::read("Cargo.toml").unwrap());
        assert_eq!(bytes, data);

        let req = TestRequest::get().uri("/Cargo.toml").to_request();
        let res = test::call_service(&srv, req).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[actix_rt::test]
    async fn test_named_file_default_service() {
        let srv = test::init_service(
            App::new().default_service(NamedFile::open("Cargo.toml").unwrap()),
        )
        .await;

        for route in ["/foobar", "/baz", "/"].iter() {
            let req = TestRequest::get().uri(route).to_request();
            let res = test::call_service(&srv, req).await;
            assert_eq!(res.status(), StatusCode::OK);

            let bytes = test::read_body(res).await;
            let data = Bytes::from(fs::read("Cargo.toml").unwrap());
            assert_eq!(bytes, data);
        }
    }

    #[actix_rt::test]
    async fn test_default_handler_named_file() {
        let st = Files::new("/", ".")
            .default_handler(NamedFile::open("Cargo.toml").unwrap())
            .new_service(())
            .await
            .unwrap();
        let req = TestRequest::with_uri("/missing").to_srv_request();
        let resp = test::call_service(&st, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = test::read_body(resp).await;
        let data = Bytes::from(fs::read("Cargo.toml").unwrap());
        assert_eq!(bytes, data);
    }

    #[actix_rt::test]
    async fn test_symlinks() {
        let srv = test::init_service(App::new().service(Files::new("test", "."))).await;

        let req = TestRequest::get()
            .uri("/test/tests/symlink-test.png")
            .to_request();
        let res = test::call_service(&srv, req).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(header::CONTENT_DISPOSITION).unwrap(),
            "inline; filename=\"symlink-test.png\""
        );
    }

    #[actix_rt::test]
    async fn test_index_with_show_files_listing() {
        let service = Files::new(".", ".")
            .index_file("lib.rs")
            .show_files_listing()
            .new_service(())
            .await
            .unwrap();

        // Serve the index if exists
        let req = TestRequest::default().uri("/src").to_srv_request();
        let resp = test::call_service(&service, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/x-rust"
        );

        // Show files listing, otherwise.
        let req = TestRequest::default().uri("/tests").to_srv_request();
        let resp = test::call_service(&service, req).await;
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );
        let bytes = test::read_body(resp).await;
        assert!(format!("{:?}", bytes).contains("/tests/test.png"));
    }
}
