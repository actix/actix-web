use std::{
    fs,
    panic::{RefUnwindSafe, UnwindSafe},
    path::Path,
};

use actix_files::{Directory, Files};
use actix_web::{dev::ServiceResponse, http::StatusCode, test, App, HttpResponse};

#[actix_web::test]
async fn directory_public_api() {
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

#[actix_web::test]
async fn nested_listing_filter_is_root_relative_and_request_aware() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("sub")).unwrap();
    for name in ["allowed.txt", "blocked.txt", ".hidden"] {
        fs::write(root.path().join("sub").join(name), "contents").unwrap();
    }
    for fallback in [false, true] {
        // Configure the filter after show_files_listing; ordering must not matter.
        let mut files = Files::new("/static", root.path())
            .show_files_listing()
            .path_filter(|path, head| {
                path == Path::new("sub")
                    || (path == Path::new("sub/allowed.txt")
                        && head.headers.contains_key("x-allow"))
            });
        if fallback {
            files = files.index_file("missing.html");
        }
        let app = test::init_service(App::new().service(files)).await;
        for allow in [false, true] {
            let mut req = test::TestRequest::with_uri("/static/sub/");
            if allow {
                req = req.insert_header(("x-allow", "yes"));
            }
            let res = test::call_service(&app, req.to_request()).await;
            assert_eq!(res.status(), StatusCode::OK);
            let body = String::from_utf8(test::read_body(res).await.to_vec()).unwrap();
            assert_eq!(body.contains("allowed.txt"), allow);
            assert!(!body.contains("blocked.txt"));
            assert!(!body.contains(".hidden"));
        }
        for (path, status) in [
            ("allowed.txt", StatusCode::OK),
            ("blocked.txt", StatusCode::NOT_FOUND),
        ] {
            let req = test::TestRequest::with_uri(&format!("/static/sub/{path}"))
                .insert_header(("x-allow", "yes"))
                .to_request();
            assert_eq!(test::call_service(&app, req).await.status(), status);
        }
    }
}

#[actix_web::test]
async fn custom_renderer_and_request_filter_remain_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let app = test::init_service(
        App::new().service(
            Files::new("/static", root.path())
                .show_files_listing()
                .files_listing_renderer(|dir, req| {
                    // Downstream struct literals and the existing two-argument callback still work.
                    let _copy = Directory {
                        base: dir.base.clone(),
                        path: dir.path.clone(),
                    };
                    Ok(ServiceResponse::new(
                        req.clone(),
                        HttpResponse::Ok().body("custom"),
                    ))
                })
                .path_filter(|_, head| head.headers.contains_key("x-allow"))
                .default_handler(|req: actix_web::dev::ServiceRequest| async {
                    Ok(req.into_response(HttpResponse::Forbidden().body("filtered")))
                }),
        ),
    )
    .await;
    let req = test::TestRequest::with_uri("/static/").to_request();
    assert_eq!(
        test::call_service(&app, req).await.status(),
        StatusCode::FORBIDDEN
    );
    let req = test::TestRequest::with_uri("/static/")
        .insert_header(("x-allow", "yes"))
        .to_request();
    let res = test::call_service(&app, req).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(test::read_body(res).await, "custom");
}
