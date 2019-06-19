use actix::System;
use actix_web::http::StatusCode;
use actix_web::web::{get, resource, HttpRequest, HttpResponse};
use actix_web::{App, HttpServer, ResponseError};
use futures::future::err;
use futures::Future;
use serde::Serialize;
use serde_json;
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::io;

#[derive(Debug, Serialize)]
struct Error {
    msg: String,
    status: u16,
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter) -> FmtResult {
        let err_json = serde_json::to_string(self).unwrap();
        write!(f, "{}", err_json)
    }
}

impl ResponseError for Error {
    fn render_response(&self) -> HttpResponse {
        HttpResponse::build(StatusCode::from_u16(self.status).unwrap()).json2(self)
    }
}

fn index(_: HttpRequest) -> impl Future<Item = HttpResponse, Error = Error> {
    err(Error {
        msg: "test".to_string(),
        status: 400,
    })
}

fn main() -> io::Result<()> {
    let sys = System::new("simple_error_response");
    let ip_address = "127.0.0.1:8000";

    HttpServer::new(|| App::new().service(resource("/").route(get().to_async(index))))
        .bind(ip_address)
        .expect("Can not bind to port 8000")
        .start();

    println!("Running server on {}", ip_address);

    sys.run()
}
