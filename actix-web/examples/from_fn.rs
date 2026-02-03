//! Shows a few of ways to use the `from_fn` middleware.

use std::{collections::HashMap, io, rc::Rc, time::Duration};

use actix_web::{
    body::MessageBody,
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    http::header::{self, HeaderValue, Range},
    middleware::{from_fn, Logger, Next},
    web::{self, Header, Query},
    App, Error, HttpResponse, HttpServer,
};
use tracing::info;

async fn noop<B>(req: ServiceRequest, next: Next<B>) -> Result<ServiceResponse<B>, Error> {
    next.call(req).await
}

async fn print_range_header<B>(
    range_header: Option<Header<Range>>,
    req: ServiceRequest,
    next: Next<B>,
) -> Result<ServiceResponse<B>, Error> {
    if let Some(Header(range)) = range_header {
        println!("Range: {range}");
    } else {
        println!("No Range header");
    }

    next.call(req).await
}

async fn mutate_body_type(
    req: ServiceRequest,
    next: Next<impl MessageBody + 'static>,
) -> Result<ServiceResponse<impl MessageBody>, Error> {
    let res = next.call(req).await?;
    Ok(res.map_into_left_body::<()>())
}

async fn mutate_body_type_with_extractors(
    string_body: String,
    query: Query<HashMap<String, String>>,
    req: ServiceRequest,
    next: Next<impl MessageBody + 'static>,
) -> Result<ServiceResponse<impl MessageBody>, Error> {
    println!("body is: {string_body}");
    println!("query string: {query:?}");

    let res = next.call(req).await?;

    Ok(res.map_body(move |_, _| string_body))
}

async fn timeout_10secs(
    req: ServiceRequest,
    next: Next<impl MessageBody + 'static>,
) -> Result<ServiceResponse<impl MessageBody>, Error> {
    match tokio::time::timeout(Duration::from_secs(10), next.call(req)).await {
        Ok(res) => res,
        Err(_err) => Err(actix_web::error::ErrorRequestTimeout("")),
    }
}

struct MyMw(bool);

impl MyMw {
    async fn mw_cb(
        &self,
        req: ServiceRequest,
        next: Next<impl MessageBody + 'static>,
    ) -> Result<ServiceResponse<impl MessageBody>, Error> {
        let mut res = match self.0 {
            true => req.into_response("short-circuited").map_into_right_body(),
            false => next.call(req).await?.map_into_left_body(),
        };

        res.headers_mut()
            .insert(header::WARNING, HeaderValue::from_static("42"));

        Ok(res)
    }

    pub fn into_middleware<S, B>(
        self,
    ) -> impl Transform<
        S,
        ServiceRequest,
        Response = ServiceResponse<impl MessageBody>,
        Error = Error,
        InitError = (),
    >
    where
        S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
        B: MessageBody + 'static,
    {
        let this = Rc::new(self);
        from_fn(move |req, next| {
            let this = Rc::clone(&this);
            async move { Self::mw_cb(&this, req, next).await }
        })
    }
}

#[actix_web::main]
async fn main() -> io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    let bind = ("127.0.0.1", 8080);
    info!("staring server at http://{}:{}", &bind.0, &bind.1);

    HttpServer::new(|| {
        App::new()
            .wrap(from_fn(noop))
            .wrap(from_fn(print_range_header))
            .wrap(from_fn(mutate_body_type))
            .wrap(from_fn(mutate_body_type_with_extractors))
            .wrap(from_fn(timeout_10secs))
            // switch bool to true to observe early response
            .wrap(MyMw(false).into_middleware())
            .wrap(Logger::default())
            .default_service(web::to(HttpResponse::Ok))
    })
    .workers(1)
    .bind(bind)?
    .run()
    .await
}
