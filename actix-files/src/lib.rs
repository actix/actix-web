//! Static file serving for Actix Web.
//!
//! Provides a non-blocking service for serving static files from disk.
//!
//! # Example
//! ```rust
//! use actix_web::App;
//! use actix_files::Files;
//!
//! let app = App::new()
//!     .service(Files::new("/static", ".").prefer_utf8(true));
//! ```

#![deny(rust_2018_idioms)]
#![warn(missing_docs, missing_debug_implementations)]

use std::io;

use actix_service::boxed::{BoxService, BoxServiceFactory};
use actix_web::{
    dev::{ServiceRequest, ServiceResponse},
    error::{BlockingError, Error, ErrorInternalServerError},
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

pub(crate) fn handle_error(err: BlockingError<io::Error>) -> Error {
    match err {
        BlockingError::Error(err) => err.into(),
        BlockingError::Canceled => ErrorInternalServerError("Unexpected error"),
    }
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
    use actix_web::{
        guard,
        http::{
            header::{self, ContentDisposition, DispositionParam, DispositionType},
            Method, StatusCode,
        },
        middleware::Compress,
        test::{self, TestRequest},
        web, App, HttpResponse, Responder,
    };
    use futures_util::future::ok;

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
        let since =
            header::HttpDate::from(SystemTime::now().add(Duration::from_secs(60)));

        let req = TestRequest::default()
            .header(header::IF_MODIFIED_SINCE, since)
            .to_http_request();
        let resp = file.respond_to(&req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
    }

    #[actix_rt::test]
    async fn test_if_modified_since_with_if_none_match() {
        let file = NamedFile::open("Cargo.toml").unwrap();
        let since =
            header::HttpDate::from(SystemTime::now().add(Duration::from_secs(60)));

        let req = TestRequest::default()
            .header(header::IF_NONE_MATCH, "miss_etag")
            .header(header::IF_MODIFIED_SINCE, since)
            .to_http_request();
        let resp = file.respond_to(&req).await.unwrap();
        assert_ne!(resp.status(), StatusCode::NOT_MODIFIED);
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
            NamedFile::from_file(File::open("Cargo.toml").unwrap(), "貨物.toml")
                .unwrap();
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

        let mut srv = test::init_service(
            App::new().service(
                Files::new("/", ".")
                    .mime_override(all_attachment)
                    .index_file("Cargo.toml"),
            ),
        )
        .await;

        let request = TestRequest::get().uri("/").to_request();
        let response = test::call_service(&mut srv, request).await;
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
        let mut srv = test::init_service(
            App::new().service(Files::new("/test", ".").index_file("Cargo.toml")),
        )
        .await;

        // Valid range header
        let request = TestRequest::get()
            .uri("/t%65st/Cargo.toml")
            .header(header::RANGE, "bytes=10-20")
            .to_request();
        let response = test::call_service(&mut srv, request).await;
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);

        // Invalid range header
        let request = TestRequest::get()
            .uri("/t%65st/Cargo.toml")
            .header(header::RANGE, "bytes=1-0")
            .to_request();
        let response = test::call_service(&mut srv, request).await;

        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    }

    #[actix_rt::test]
    async fn test_named_file_content_range_headers() {
        let srv = test::start(|| App::new().service(Files::new("/", ".")));

        // Valid range header
        let response = srv
            .get("/tests/test.binary")
            .header(header::RANGE, "bytes=10-20")
            .send()
            .await
            .unwrap();
        let content_range = response.headers().get(header::CONTENT_RANGE).unwrap();
        assert_eq!(content_range.to_str().unwrap(), "bytes 10-20/100");

        // Invalid range header
        let response = srv
            .get("/tests/test.binary")
            .header(header::RANGE, "bytes=10-5")
            .send()
            .await
            .unwrap();
        let content_range = response.headers().get(header::CONTENT_RANGE).unwrap();
        assert_eq!(content_range.to_str().unwrap(), "bytes */100");
    }

    #[actix_rt::test]
    async fn test_named_file_content_length_headers() {
        let srv = test::start(|| App::new().service(Files::new("/", ".")));

        // Valid range header
        let response = srv
            .get("/tests/test.binary")
            .header(header::RANGE, "bytes=10-20")
            .send()
            .await
            .unwrap();
        let content_length = response.headers().get(header::CONTENT_LENGTH).unwrap();
        assert_eq!(content_length.to_str().unwrap(), "11");

        // Valid range header, starting from 0
        let response = srv
            .get("/tests/test.binary")
            .header(header::RANGE, "bytes=0-20")
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
        let srv = test::start(|| App::new().service(Files::new("/", ".")));

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
        let mut srv = test::init_service(
            App::new().service(Files::new("/", ".").index_file("Cargo.toml")),
        )
        .await;
        let request = TestRequest::get()
            .uri("/tests/test%20space.binary")
            .to_request();
        let response = test::call_service(&mut srv, request).await;
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = test::read_body(response).await;
        let data = web::Bytes::from(fs::read("tests/test space.binary").unwrap());
        assert_eq!(bytes, data);
    }

    #[actix_rt::test]
    async fn test_files_not_allowed() {
        let mut srv = test::init_service(App::new().service(Files::new("/", "."))).await;

        let req = TestRequest::default()
            .uri("/Cargo.toml")
            .method(Method::POST)
            .to_request();

        let resp = test::call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);

        let mut srv = test::init_service(App::new().service(Files::new("/", "."))).await;
        let req = TestRequest::default()
            .method(Method::PUT)
            .uri("/Cargo.toml")
            .to_request();
        let resp = test::call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[actix_rt::test]
    async fn test_files_guards() {
        let mut srv = test::init_service(
            App::new().service(Files::new("/", ".").use_guards(guard::Post())),
        )
        .await;

        let req = TestRequest::default()
            .uri("/Cargo.toml")
            .method(Method::POST)
            .to_request();

        let resp = test::call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_named_file_content_encoding() {
        let mut srv = test::init_service(App::new().wrap(Compress::default()).service(
            web::resource("/").to(|| async {
                NamedFile::open("Cargo.toml")
                    .unwrap()
                    .set_content_encoding(header::ContentEncoding::Identity)
            }),
        ))
        .await;

        let request = TestRequest::get()
            .uri("/")
            .header(header::ACCEPT_ENCODING, "gzip")
            .to_request();
        let res = test::call_service(&mut srv, request).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert!(!res.headers().contains_key(header::CONTENT_ENCODING));
    }

    #[actix_rt::test]
    async fn test_named_file_content_encoding_gzip() {
        let mut srv = test::init_service(App::new().wrap(Compress::default()).service(
            web::resource("/").to(|| async {
                NamedFile::open("Cargo.toml")
                    .unwrap()
                    .set_content_encoding(header::ContentEncoding::Gzip)
            }),
        ))
        .await;

        let request = TestRequest::get()
            .uri("/")
            .header(header::ACCEPT_ENCODING, "gzip")
            .to_request();
        let res = test::call_service(&mut srv, request).await;
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
        let mut srv = test::init_service(
            App::new().service(Files::new("/", ".").show_files_listing()),
        )
        .await;
        let req = TestRequest::with_uri("/missing").to_request();

        let resp = test::call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let mut srv = test::init_service(App::new().service(Files::new("/", "."))).await;

        let req = TestRequest::default().to_request();
        let resp = test::call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let mut srv = test::init_service(
            App::new().service(Files::new("/", ".").show_files_listing()),
        )
        .await;
        let req = TestRequest::with_uri("/tests").to_request();
        let resp = test::call_service(&mut srv, req).await;
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );

        let bytes = test::read_body(resp).await;
        assert!(format!("{:?}", bytes).contains("/tests/test.png"));
    }

    #[actix_rt::test]
    async fn test_redirect_to_slash_directory() {
        // should not redirect if no index
        let mut srv = test::init_service(
            App::new().service(Files::new("/", ".").redirect_to_slash_directory()),
        )
        .await;
        let req = TestRequest::with_uri("/tests").to_request();
        let resp = test::call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        // should redirect if index present
        let mut srv = test::init_service(
            App::new().service(
                Files::new("/", ".")
                    .index_file("test.png")
                    .redirect_to_slash_directory(),
            ),
        )
        .await;
        let req = TestRequest::with_uri("/tests").to_request();
        let resp = test::call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::FOUND);

        // should not redirect if the path is wrong
        let req = TestRequest::with_uri("/not_existing").to_request();
        let resp = test::call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[actix_rt::test]
    async fn test_static_files_bad_directory() {
        let _st: Files = Files::new("/", "missing");
        let _st: Files = Files::new("/", "Cargo.toml");
    }

    #[actix_rt::test]
    async fn test_default_handler_file_missing() {
        let mut st = Files::new("/", ".")
            .default_handler(|req: ServiceRequest| {
                ok(req.into_response(HttpResponse::Ok().body("default content")))
            })
            .new_service(())
            .await
            .unwrap();
        let req = TestRequest::with_uri("/missing").to_srv_request();

        let resp = test::call_service(&mut st, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = test::read_body(resp).await;
        assert_eq!(bytes, web::Bytes::from_static(b"default content"));
    }

    //     #[actix_rt::test]
    //     async fn test_serve_index() {
    //         let st = Files::new(".").index_file("test.binary");
    //         let req = TestRequest::default().uri("/tests").finish();

    //         let resp = st.handle(&req).respond_to(&req).unwrap();
    //         let resp = resp.as_msg();
    //         assert_eq!(resp.status(), StatusCode::OK);
    //         assert_eq!(
    //             resp.headers()
    //                 .get(header::CONTENT_TYPE)
    //                 .expect("content type"),
    //             "application/octet-stream"
    //         );
    //         assert_eq!(
    //             resp.headers()
    //                 .get(header::CONTENT_DISPOSITION)
    //                 .expect("content disposition"),
    //             "attachment; filename=\"test.binary\""
    //         );

    //         let req = TestRequest::default().uri("/tests/").finish();
    //         let resp = st.handle(&req).respond_to(&req).unwrap();
    //         let resp = resp.as_msg();
    //         assert_eq!(resp.status(), StatusCode::OK);
    //         assert_eq!(
    //             resp.headers().get(header::CONTENT_TYPE).unwrap(),
    //             "application/octet-stream"
    //         );
    //         assert_eq!(
    //             resp.headers().get(header::CONTENT_DISPOSITION).unwrap(),
    //             "attachment; filename=\"test.binary\""
    //         );

    //         // nonexistent index file
    //         let req = TestRequest::default().uri("/tests/unknown").finish();
    //         let resp = st.handle(&req).respond_to(&req).unwrap();
    //         let resp = resp.as_msg();
    //         assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    //         let req = TestRequest::default().uri("/tests/unknown/").finish();
    //         let resp = st.handle(&req).respond_to(&req).unwrap();
    //         let resp = resp.as_msg();
    //         assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    //     }

    //     #[actix_rt::test]
    //     async fn test_serve_index_nested() {
    //         let st = Files::new(".").index_file("mod.rs");
    //         let req = TestRequest::default().uri("/src/client").finish();
    //         let resp = st.handle(&req).respond_to(&req).unwrap();
    //         let resp = resp.as_msg();
    //         assert_eq!(resp.status(), StatusCode::OK);
    //         assert_eq!(
    //             resp.headers().get(header::CONTENT_TYPE).unwrap(),
    //             "text/x-rust"
    //         );
    //         assert_eq!(
    //             resp.headers().get(header::CONTENT_DISPOSITION).unwrap(),
    //             "inline; filename=\"mod.rs\""
    //         );
    //     }

    //     #[actix_rt::test]
    //     fn integration_serve_index() {
    //         let mut srv = test::TestServer::with_factory(|| {
    //             App::new().handler(
    //                 "test",
    //                 Files::new(".").index_file("Cargo.toml"),
    //             )
    //         });

    //         let request = srv.get().uri(srv.url("/test")).finish().unwrap();
    //         let response = srv.execute(request.send()).unwrap();
    //         assert_eq!(response.status(), StatusCode::OK);
    //         let bytes = srv.execute(response.body()).unwrap();
    //         let data = Bytes::from(fs::read("Cargo.toml").unwrap());
    //         assert_eq!(bytes, data);

    //         let request = srv.get().uri(srv.url("/test/")).finish().unwrap();
    //         let response = srv.execute(request.send()).unwrap();
    //         assert_eq!(response.status(), StatusCode::OK);
    //         let bytes = srv.execute(response.body()).unwrap();
    //         let data = Bytes::from(fs::read("Cargo.toml").unwrap());
    //         assert_eq!(bytes, data);

    //         // nonexistent index file
    //         let request = srv.get().uri(srv.url("/test/unknown")).finish().unwrap();
    //         let response = srv.execute(request.send()).unwrap();
    //         assert_eq!(response.status(), StatusCode::NOT_FOUND);

    //         let request = srv.get().uri(srv.url("/test/unknown/")).finish().unwrap();
    //         let response = srv.execute(request.send()).unwrap();
    //         assert_eq!(response.status(), StatusCode::NOT_FOUND);
    //     }

    //     #[actix_rt::test]
    //     fn integration_percent_encoded() {
    //         let mut srv = test::TestServer::with_factory(|| {
    //             App::new().handler(
    //                 "test",
    //                 Files::new(".").index_file("Cargo.toml"),
    //             )
    //         });

    //         let request = srv
    //             .get()
    //             .uri(srv.url("/test/%43argo.toml"))
    //             .finish()
    //             .unwrap();
    //         let response = srv.execute(request.send()).unwrap();
    //         assert_eq!(response.status(), StatusCode::OK);
    //     }
}
