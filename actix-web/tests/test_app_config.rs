use std::{
    io,
    net::{SocketAddr, TcpListener},
    time::Duration,
};

use actix_http::HttpService;
use actix_server::Server;
use actix_service::{map_config, ServiceFactory as _};
use actix_web::{
    dev::AppConfig,
    http::{header, StatusCode},
    rt::{self, net::TcpStream, time::timeout},
    test::{call_service, read_body_json, TestRequest},
    web, App, HttpRequest, HttpResponse,
};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

async fn config_response(req: HttpRequest) -> HttpResponse {
    let config = req.app_config();
    let info = req.connection_info();

    HttpResponse::Ok().json(json!({
        "secure": config.secure(),
        "host": config.host(),
        "local_addr": config.local_addr().to_string(),
        "scheme": info.scheme(),
        "connection_host": info.host(),
        "url": req.url_for("item", ["42"]).unwrap().as_str(),
        "static_url": req.url_for_static("index").unwrap().as_str(),
    }))
}

fn configure_app(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/").name("index").to(config_response))
        .service(web::resource("/items/{id}").name("item"));
}

#[actix_rt::test]
async fn custom_app_config_used_for_connection_info_and_urls() {
    for (secure, scheme, port) in [(false, "http", 8081), (true, "https", 8443)] {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let host = format!("example.com:{port}");
        let config = AppConfig::new(secure, host.clone(), addr);
        let app = map_config(App::new().configure(configure_app), move |()| {
            config.clone()
        })
        .new_service(())
        .await
        .unwrap();

        // No Host header or URI authority: connection information must use the app config.
        let res = call_service(&app, TestRequest::with_uri("/").to_request()).await;
        assert_eq!(res.status(), StatusCode::OK);
        let body: Value = read_body_json(res).await;
        assert_eq!(
            body,
            json!({
                "secure": secure,
                "host": host,
                "local_addr": addr.to_string(),
                "scheme": scheme,
                "connection_host": host,
                "url": format!("{scheme}://{host}/items/42"),
                "static_url": format!("{scheme}://{host}/"),
            }),
        );
    }
}

#[actix_rt::test]
async fn request_information_overrides_app_config() {
    let addr = SocketAddr::from(([127, 0, 0, 1], 8443));
    let config = AppConfig::new(true, "configured.example:8443".to_owned(), addr);
    let app = map_config(App::new().configure(configure_app), move |()| {
        config.clone()
    })
    .new_service(())
    .await
    .unwrap();

    let cases = [
        (
            TestRequest::default().insert_header((header::HOST, "header.example:9090")),
            "https",
            "header.example:9090",
        ),
        (
            TestRequest::with_uri("http://uri.example:8081/"),
            "http",
            "uri.example:8081",
        ),
        (
            TestRequest::with_uri("http://uri.example:8081/")
                .insert_header((header::HOST, "header.example:9090")),
            "http",
            "header.example:9090",
        ),
        (
            TestRequest::with_uri("https://uri.example/")
                .insert_header((header::HOST, "header.example"))
                .insert_header(("x-forwarded-host", "proxy.example:8082"))
                .insert_header(("x-forwarded-proto", "http")),
            "http",
            "proxy.example:8082",
        ),
        (
            TestRequest::with_uri("https://uri.example/")
                .insert_header((header::HOST, "header.example"))
                .insert_header(("x-forwarded-host", "proxy.example"))
                .insert_header(("x-forwarded-proto", "https"))
                .insert_header((header::FORWARDED, "host=forwarded.example:8083;proto=http")),
            "http",
            "forwarded.example:8083",
        ),
    ];

    for (req, scheme, host) in cases {
        let res = call_service(&app, req.to_request()).await;
        assert_eq!(res.status(), StatusCode::OK);
        let body: Value = read_body_json(res).await;
        assert_eq!(
            body,
            json!({
                "secure": true,
                "host": "configured.example:8443",
                "local_addr": addr.to_string(),
                "scheme": scheme,
                "connection_host": host,
                "url": format!("{scheme}://{host}/items/42"),
                "static_url": format!("{scheme}://{host}/"),
            }),
        );
    }
}

#[actix_rt::test]
async fn multiple_listeners_keep_separate_app_configs() {
    let listeners = [
        TcpListener::bind("127.0.0.1:0").unwrap(),
        TcpListener::bind("127.0.0.1:0").unwrap(),
    ];
    let addrs = listeners.each_ref().map(|lst| lst.local_addr().unwrap());
    let mut server = Server::build().workers(1).disable_signals();

    for (idx, listener) in listeners.into_iter().enumerate() {
        let addr = addrs[idx];
        server = server
            .listen(format!("app-{idx}"), listener, move || {
                HttpService::build()
                    .local_addr(addr)
                    .h1(map_config(App::new().configure(configure_app), move |()| {
                        AppConfig::new(false, addr.to_string(), addr)
                    }))
                    .tcp()
            })
            .unwrap();
    }

    let server = server.run();
    let handle = server.handle();
    let server_task = rt::spawn(server);
    let timeout_duration = Duration::from_secs(10);
    let responses = timeout(timeout_duration, async {
        let mut responses = Vec::new();

        for addr in [addrs[0], addrs[1], addrs[0]] {
            let mut stream = TcpStream::connect(addr).await?;
            // HTTP/1.0 permits omitting Host and closes the connection after the response.
            stream.write_all(b"GET / HTTP/1.0\r\n\r\n").await?;
            let mut response = String::new();
            stream.read_to_string(&mut response).await?;
            responses.push((addr, response));
        }

        Ok::<_, io::Error>(responses)
    })
    .await;

    // Stop the server before checking responses so failed assertions also release its worker.
    timeout(timeout_duration, handle.stop(false))
        .await
        .expect("server shutdown timed out");
    timeout(timeout_duration, server_task)
        .await
        .expect("server task timed out")
        .expect("server task panicked")
        .expect("server failed");

    for (addr, response) in responses.expect("requests timed out").unwrap() {
        assert!(response.starts_with("HTTP/1.0 200 OK\r\n"), "{response}");
        let (_, body) = response.split_once("\r\n\r\n").unwrap();
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(
            body,
            json!({
                "secure": false,
                "host": addr.to_string(),
                "local_addr": addr.to_string(),
                "scheme": "http",
                "connection_host": addr.to_string(),
                "url": format!("http://{addr}/items/42"),
                "static_url": format!("http://{addr}/"),
            }),
        );
    }
}
