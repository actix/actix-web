use actix_web::{middleware::DefaultHeaders, test, web, App, HttpResponse};

#[actix_web::test]
async fn group_boxes_concrete_response_bodies_between_middleware_layers() {
    use actix_web::{
        body::BoxBody,
        dev::{ServiceRequest, ServiceResponse},
        middleware::{from_fn, Next},
        Error,
    };

    async fn string_body(
        req: ServiceRequest,
        next: Next<BoxBody>,
    ) -> Result<ServiceResponse<String>, Error> {
        let res = next.call(req).await?;
        Ok(res.map_body(|_, _| "middleware body".to_owned()))
    }

    // This signature checks the public middleware boundary: the String returned
    // by the inner middleware is erased before the outer middleware receives it.
    async fn boxed_body(
        req: ServiceRequest,
        next: Next<BoxBody>,
    ) -> Result<ServiceResponse<BoxBody>, Error> {
        next.call(req).await
    }

    let app = test::init_service(
        App::new().service(
            web::group()
                .wrap(from_fn(string_body))
                .wrap(from_fn(boxed_body))
                .route("/body", web::get().to(HttpResponse::Accepted)),
        ),
    )
    .await;
    let res = test::call_service(&app, test::TestRequest::with_uri("/body").to_request()).await;
    assert_eq!(res.status(), actix_web::http::StatusCode::ACCEPTED);
    assert_eq!(test::read_body(res).await, "middleware body");
}

#[actix_web::test]
async fn group_middleware_extracts_parent_data_before_child_scope_data_is_added() {
    use actix_web::{
        body::MessageBody,
        dev::{ServiceRequest, ServiceResponse},
        http::header::{HeaderName, HeaderValue},
        middleware::{from_fn, Next},
        Error,
    };

    async fn report_data(
        data: web::Data<u32>,
        req: ServiceRequest,
        next: Next<impl MessageBody>,
    ) -> Result<ServiceResponse<impl MessageBody>, Error> {
        let mut res = next.call(req).await?;
        res.headers_mut().insert(
            HeaderName::from_static("x-middleware-data"),
            HeaderValue::from_str(&data.to_string()).unwrap(),
        );
        Ok(res)
    }

    for grouped in [true, false] {
        let scope = web::scope("/child").app_data(web::Data::new(2u32)).route(
            "/value",
            web::get().to(|data: web::Data<u32>| async move { data.to_string() }),
        );
        let app = App::new().app_data(web::Data::new(1u32));
        let app = if grouped {
            app.service(web::group().wrap(from_fn(report_data)).service(scope))
        } else {
            app.service(scope.wrap(from_fn(report_data)))
        };
        let app = test::init_service(app).await;
        let res = test::call_service(
            &app,
            test::TestRequest::with_uri("/child/value").to_request(),
        )
        .await;
        assert_eq!(
            res.headers().get("x-middleware-data").unwrap(),
            if grouped { "1" } else { "2" }
        );
        assert_eq!(test::read_body(res).await, "2");
    }
}

#[actix_web::test]
async fn group_middleware_cannot_change_the_selected_child_by_rewriting_the_uri() {
    use actix_web::{
        body::MessageBody,
        dev::{ServiceRequest, ServiceResponse, Url},
        middleware::{from_fn, Next},
        Error, HttpRequest,
    };

    async fn rewrite(
        mut req: ServiceRequest,
        next: Next<impl MessageBody>,
    ) -> Result<ServiceResponse<impl MessageBody>, Error> {
        let uri = "/target".parse::<actix_web::http::Uri>().unwrap();
        req.match_info_mut().set(Url::new(uri.clone()));
        req.head_mut().uri = uri;
        next.call(req).await
    }

    for grouped in [true, false] {
        let children = (
            web::resource("/original")
                .to(|req: HttpRequest| async move { format!("original:{}", req.path()) }),
            web::resource("/target")
                .to(|req: HttpRequest| async move { format!("target:{}", req.path()) }),
        );
        let app = App::new();
        let app = if grouped {
            app.service(web::group().wrap(from_fn(rewrite)).service(children))
        } else {
            app.service(web::scope("").wrap(from_fn(rewrite)).service(children))
        };
        let app = test::init_service(app).await;
        let body =
            test::call_and_read_body(&app, test::TestRequest::with_uri("/original").to_request())
                .await;
        assert_eq!(
            body,
            if grouped {
                "original:/target"
            } else {
                "target:/target"
            }
        );
    }
}

#[cfg(feature = "experimental-introspection")]
#[actix_web::test]
async fn groups_report_each_resource_once_with_its_parent_prefix() {
    async fn report(tree: web::Data<actix_web::introspection::IntrospectionTree>) -> String {
        tree.report_as_json()
    }
    let app = test::init_service(
        App::new()
            .service(
                web::scope("/api").service(
                    web::group()
                        .wrap(DefaultHeaders::new().add(("x-group", "group")))
                        .service(
                            web::group().service(
                                web::resource("/item")
                                    .name("item")
                                    .route(web::get().to(HttpResponse::Ok)),
                            ),
                        ),
                ),
            )
            .route("/report", web::get().to(report)),
    )
    .await;
    let body =
        test::call_and_read_body(&app, test::TestRequest::with_uri("/report").to_request()).await;
    let items = serde_json::from_slice::<Vec<serde_json::Value>>(&body).unwrap();
    let matching = items
        .iter()
        .filter(|item| item["resource_name"] == "item")
        .collect::<Vec<_>>();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0]["full_path"], "/api/item");
    assert_eq!(matching[0]["resource_type"], "resource");
}

struct VisitCounter;

impl<S> actix_web::dev::Transform<S, actix_web::dev::ServiceRequest> for VisitCounter
where
    S: actix_web::dev::Service<
            actix_web::dev::ServiceRequest,
            Response = actix_web::dev::ServiceResponse,
            Error = actix_web::Error,
        > + 'static,
    S::Future: 'static,
{
    type Response = actix_web::dev::ServiceResponse;
    type Error = actix_web::Error;
    type InitError = ();
    type Transform = actix_service::boxed::BoxService<
        actix_web::dev::ServiceRequest,
        Self::Response,
        Self::Error,
    >;
    type Future = std::future::Ready<Result<Self::Transform, ()>>;

    fn new_transform(&self, service: S) -> Self::Future {
        let visits = std::cell::Cell::new(0);
        std::future::ready(Ok(actix_service::boxed::service(actix_service::apply_fn(
            service,
            move |mut req: actix_web::dev::ServiceRequest, service: &S| {
                use actix_web::http::header::{HeaderName, HeaderValue};
                visits.set(visits.get() + 1);
                req.headers_mut().insert(
                    HeaderName::from_static("x-visits"),
                    HeaderValue::from_str(&visits.get().to_string()).unwrap(),
                );
                service.call(req)
            },
        ))))
    }
}

#[actix_web::test]
async fn non_clone_middleware_has_separate_state_for_each_child() {
    async fn visits(req: actix_web::HttpRequest) -> String {
        req.headers()
            .get("x-visits")
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned()
    }
    let app = test::init_service(
        App::new().service(
            web::group()
                .wrap(VisitCounter)
                .route("/a", web::get().to(visits))
                .route("/b", web::get().to(visits))
                .service(
                    web::scope("/scope")
                        .route("/a", web::get().to(visits))
                        .route("/b", web::get().to(visits)),
                ),
        ),
    )
    .await;
    for (path, expected) in [
        ("/a", "1"),
        ("/a", "2"),
        ("/b", "1"),
        ("/b", "2"),
        ("/a", "3"),
        ("/scope/a", "1"),
        ("/scope/b", "2"),
        ("/scope/a", "3"),
    ] {
        assert_eq!(
            test::call_and_read_body(&app, test::TestRequest::with_uri(path).to_request()).await,
            expected
        );
    }
}

#[actix_web::test]
async fn group_children_keep_resource_and_scope_default_behavior() {
    let app = test::init_service(
        App::new()
            .service(
                web::group()
                    .wrap(DefaultHeaders::new().add(("x-group", "selected")))
                    .service(
                        web::resource("/missing")
                            .to(|| async { HttpResponse::NotFound().body("handler") }),
                    )
                    .service(web::resource("/method").route(web::get().to(HttpResponse::Ok)))
                    .service(web::scope("/scope").default_service(web::to(|| async {
                        HttpResponse::NotFound().body("scope")
                    }))),
            )
            .service(web::resource("/missing").to(|| async { "sibling" }))
            .service(web::resource("/scope/unknown").to(|| async { "scope sibling" }))
            .default_service(web::to(|| async {
                HttpResponse::NotFound().body("parent")
            })),
    )
    .await;

    for (path, expected, grouped) in [
        ("/missing", "handler", true),
        ("/scope/unknown", "scope", true),
        ("/unknown", "parent", false),
    ] {
        let res = test::call_service(&app, test::TestRequest::with_uri(path).to_request()).await;
        assert_eq!(res.status(), actix_web::http::StatusCode::NOT_FOUND);
        assert_eq!(res.headers().contains_key("x-group"), grouped);
        assert_eq!(test::read_body(res).await, expected);
    }
    let res = test::call_service(&app, test::TestRequest::post().uri("/method").to_request()).await;
    assert_eq!(
        res.status(),
        actix_web::http::StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(res.headers().get("x-group").unwrap(), "selected");
}

#[actix_web::test]
async fn groups_preserve_nested_scope_paths_data_and_url_generation() {
    let app = test::init_service(
        App::new().service(
            web::scope("/api").service(
                web::group().service(
                    web::scope("/users")
                        .app_data(web::Data::new(42u32))
                        .service(web::group().service(web::resource("/{id}").name("user").to(
                            |req: actix_web::HttpRequest, data: web::Data<u32>| async move {
                                format!(
                                    "{}|{}|{}|{}",
                                    req.match_info().get("id").unwrap(),
                                    req.match_pattern().unwrap(),
                                    req.url_for("user", ["8"]).unwrap(),
                                    **data
                                )
                            },
                        ))),
                ),
            ),
        ),
    )
    .await;
    let req = test::TestRequest::with_uri("/api/users/7")
        .insert_header(("host", "example.com"))
        .to_request();
    assert_eq!(
        test::call_and_read_body(&app, req).await,
        "7|/api/users/{id}|http://example.com/api/users/8|42"
    );
}

async fn stamp(
    name: &'static str,
    mut req: actix_web::dev::ServiceRequest,
    next: actix_web::middleware::Next<impl actix_web::body::MessageBody>,
) -> Result<actix_web::dev::ServiceResponse<impl actix_web::body::MessageBody>, actix_web::Error> {
    use actix_web::http::header::{HeaderName, HeaderValue};
    req.headers_mut().append(
        HeaderName::from_static("x-order"),
        HeaderValue::from_static(name),
    );
    let mut res = next.call(req).await?;
    res.headers_mut().append(
        HeaderName::from_static("x-order"),
        HeaderValue::from_static(name),
    );
    Ok(res)
}

async fn outer(
    req: actix_web::dev::ServiceRequest,
    next: actix_web::middleware::Next<impl actix_web::body::MessageBody>,
) -> Result<actix_web::dev::ServiceResponse<impl actix_web::body::MessageBody>, actix_web::Error> {
    stamp("outer", req, next).await
}

async fn inner(
    req: actix_web::dev::ServiceRequest,
    next: actix_web::middleware::Next<impl actix_web::body::MessageBody>,
) -> Result<actix_web::dev::ServiceResponse<impl actix_web::body::MessageBody>, actix_web::Error> {
    stamp("inner", req, next).await
}

#[actix_web::test]
async fn nested_groups_preserve_request_and_response_middleware_order() {
    use actix_web::middleware::from_fn;
    let app = test::init_service(
        App::new().service(
            web::group()
                .service(web::group().wrap(from_fn(inner)).route(
                    "/order",
                    web::get().to(|req: actix_web::HttpRequest| async move {
                        req.headers()
                            .get_all("x-order")
                            .map(|v| v.to_str().unwrap())
                            .collect::<Vec<_>>()
                            .join(",")
                    }),
                ))
                .wrap(from_fn(inner))
                .wrap(from_fn(outer)),
        ),
    )
    .await;
    let res = test::call_service(&app, test::TestRequest::with_uri("/order").to_request()).await;
    assert_eq!(
        res.headers()
            .get_all("x-order")
            .map(|v| v.to_str().unwrap())
            .collect::<Vec<_>>(),
        ["inner", "inner", "outer"]
    );
    assert_eq!(test::read_body(res).await, "outer,inner,inner");
}

#[actix_web::test]
async fn group_routes_preserve_method_guards_and_parent_default() {
    let app = test::init_service(
        App::new()
            .service(web::group().route("/item", web::get().to(|| async { "get" })))
            .service(web::group().route("/item", web::post().to(|| async { "post" })))
            .default_service(web::to(|| async {
                HttpResponse::NotFound().body("default")
            })),
    )
    .await;

    for (method, expected) in [
        (actix_web::http::Method::GET, "get"),
        (actix_web::http::Method::POST, "post"),
        (actix_web::http::Method::DELETE, "default"),
    ] {
        let req = test::TestRequest::with_uri("/item")
            .method(method)
            .to_request();
        assert_eq!(test::call_and_read_body(&app, req).await, expected);
    }
}

#[actix_web::test]
async fn groups_wrap_only_their_children_without_claiming_siblings() {
    for reverse in [false, true] {
        let first = web::group()
            .wrap(DefaultHeaders::new().add(("x-group", "first")))
            .service(web::resource("/first").to(HttpResponse::Ok));
        let second = web::group()
            .wrap(DefaultHeaders::new().add(("x-group", "second")))
            .service(web::resource("/second").to(HttpResponse::Ok));
        let groups = if reverse {
            vec![second, first]
        } else {
            vec![first, second]
        };
        let app = test::init_service(
            App::new()
                .service(groups)
                .service(web::resource("/public").to(HttpResponse::Ok)),
        )
        .await;

        for (path, group) in [
            ("/first", Some("first")),
            ("/second", Some("second")),
            ("/public", None),
        ] {
            let res =
                test::call_service(&app, test::TestRequest::with_uri(path).to_request()).await;
            assert!(res.status().is_success());
            assert_eq!(
                res.headers()
                    .get("x-group")
                    .map(|value| value.to_str().unwrap()),
                group
            );
        }
    }
}
