#![allow(dead_code)]
extern crate actix;
extern crate actix_http;
extern crate tokio_core;
extern crate env_logger;

use std::net;
use std::str::FromStr;

use actix::prelude::*;
use actix_http::*;

struct MyRoute {req: Option<HttpRequest>}

impl Actor for MyRoute {
    type Context = HttpContext<Self>;
}

impl Route for MyRoute {
    type State = ();

    fn request(req: HttpRequest,
               payload: Option<Payload>,
               ctx: &mut HttpContext<Self>) -> HttpResponse<Self>
    {
        if let Some(pl) = payload {
            ctx.add_stream(pl);
            HttpResponse::Stream(MyRoute{req: Some(req)})
        } else {
            HttpResponse::Reply(req, httpcodes::HTTPOk)
        }
    }
}

impl ResponseType<PayloadItem> for MyRoute {
    type Item = ();
    type Error = ();
}

impl StreamHandler<PayloadItem, ()> for MyRoute {}

impl Handler<PayloadItem> for MyRoute {
    fn handle(&mut self, msg: PayloadItem, ctx: &mut HttpContext<Self>)
              -> Response<Self, PayloadItem>
    {
        println!("CHUNK: {:?}", msg);
        if let Some(req) = self.req.take() {
            ctx.start(httpcodes::HTTPOk.into_response(req));
            ctx.write_eof();
        }

        Response::Empty()
    }
}


fn main() {
    let _ = env_logger::init();

    let sys = actix::System::new("http-example".to_owned());

    let mut routes = RoutingMap::default();

    let mut app = HttpApplication::no_state();
    app.add("/test")
        .get::<MyRoute>()
        .post::<MyRoute>();

    routes.add("/blah", app);

    routes.add_resource("/test")
        .post::<MyRoute>();

    let http = HttpServer::new(routes);
    http.serve::<()>(
        &net::SocketAddr::from_str("127.0.0.1:9080").unwrap()).unwrap();

    println!("starting");
    let _ = sys.run();
}
