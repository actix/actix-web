use std::{
    fs,
    panic::{RefUnwindSafe, UnwindSafe},
    path::Path,
};

use actix_files::{Directory, Files};
use actix_web::{
    dev::{RequestHead, ServiceResponse},
    http::StatusCode,
    test::{self, TestRequest},
    App, HttpResponse,
};

#[test]
fn directory_public_api() {
    fn assert_traits<T: Send + Sync + UnwindSafe + RefUnwindSafe + Unpin>() {}
    assert_traits::<Directory>();

    let dir = Directory {
        base: "root".into(),
        path: "root/sub".into(),
    };
    let dir = Directory {
        path: "root/other".into(),
        ..dir
    };
    let Directory { base, path } = dir;

    assert_eq!(base, Path::new("root"));
    assert_eq!(path, Path::new("root/other"));
}

/// Paths are relative to the serving root, for both requests and listing entries.
fn listing_filter(path: &Path, head: &RequestHead) -> bool {
    let allow = head.headers.contains_key("x-allow");

    // Listing requests for the root and `sub` pass; the `denied` directory does not.
    path.as_os_str().is_empty()
        || path == Path::new("sub")
        // Dot-files are let through so the listing's own hidden-file rule is exercised.
        || path == Path::new(".hidden")
        || path == Path::new("sub/.hidden")
        || (allow && (path == Path::new("allowed.txt") || path == Path::new("sub/allowed.txt")))
}

#[actix_web::test]
async fn default_listing_applies_path_filter() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("sub")).unwrap();
    // Rejected by the filter; stays empty since the listing does not recurse.
    fs::create_dir(root.path().join("denied")).unwrap();
    for dir in [root.path().to_path_buf(), root.path().join("sub")] {
        for name in ["allowed.txt", "blocked.txt", ".hidden"] {
            fs::write(dir.join(name), "contents").unwrap();
        }
    }

    for (filter_first, fallback) in [(false, false), (false, true), (true, false), (true, true)] {
        // Builder order must not matter.
        let files = if filter_first {
            Files::new("/static", root.path())
                .path_filter(listing_filter)
                .show_files_listing()
        } else {
            Files::new("/static", root.path())
                .show_files_listing()
                .path_filter(listing_filter)
        };
        // A missing index file falls back to the listing, which must still be filtered.
        let files = if fallback {
            files.index_file("missing.html")
        } else {
            files
        };
        let app = test::init_service(App::new().service(files)).await;

        for (uri, is_root) in [("/static/", true), ("/static/sub/", false)] {
            for allow in [false, true] {
                let mut req = TestRequest::with_uri(uri);
                if allow {
                    req = req.insert_header(("x-allow", "yes"));
                }
                let res = test::call_service(&app, req.to_request()).await;
                assert_eq!(res.status(), StatusCode::OK);
                let body = String::from_utf8(test::read_body(res).await.to_vec()).unwrap();

                assert_eq!(
                    body.contains("allowed.txt"),
                    allow,
                    "{uri} allow={allow}: {body}"
                );
                assert!(!body.contains("blocked.txt"), "{uri}: {body}");
                assert!(!body.contains(".hidden"), "{uri}: {body}");
                if is_root {
                    assert!(body.contains("sub/"), "{body}");
                    assert!(!body.contains("denied"), "{body}");
                }
            }
        }
    }
}

#[actix_web::test]
async fn custom_renderer_keeps_two_argument_callback() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("blocked.txt"), "contents").unwrap();

    let app = test::init_service(
        App::new().service(
            Files::new("/static", root.path())
                .show_files_listing()
                // Custom renderers stay responsible for their own listing contents.
                .path_filter(|path, _| path.as_os_str().is_empty())
                .files_listing_renderer(|_dir, req| {
                    Ok(ServiceResponse::new(
                        req.clone(),
                        HttpResponse::Ok().body("custom"),
                    ))
                }),
        ),
    )
    .await;

    let req = TestRequest::with_uri("/static/").to_request();
    let res = test::call_service(&app, req).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(test::read_body(res).await, "custom");
}
