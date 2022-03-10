use std::io;

use actix_http::{
    body::{BodyStream, MessageBody},
    header, Error, HttpMessage, HttpService, Request, Response, StatusCode,
};

async fn handle_request(mut req: Request) -> Result<Response<impl MessageBody>, Error> {
    let mut res = Response::build(StatusCode::OK);

    if let Some(ct) = req.headers().get(header::CONTENT_TYPE) {
        res.insert_header((header::CONTENT_TYPE, ct));
    }

    // echo request payload stream as (chunked) response body
    let res = res.message_body(BodyStream::new(req.payload().take()))?;

    Ok(res)
}

#[actix_rt::main]
async fn main() -> io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    actix_server::Server::build()
        .bind("echo", ("127.0.0.1", 8080), || {
            HttpService::build()
                // handles HTTP/1.1 only
                .h1(handle_request)
                // No TLS
                .tcp()
        })?
        .run()
        .await
}
