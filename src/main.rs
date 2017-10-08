#![allow(dead_code, unused_variables)]
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
               ctx: &mut HttpContext<Self>) -> Reply<Self>
    {
        if let Some(pl) = payload {
            ctx.add_stream(pl);
            Reply::stream(MyRoute{req: Some(req)})
        } else {
            Reply::with(req, httpcodes::HTTPOk)
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
            ctx.start(httpcodes::HTTPOk.response(req));
            ctx.write_eof();
        }

        Self::empty()
    }
}

struct MyWS {}

impl Actor for MyWS {
    type Context = HttpContext<Self>;
}

impl Route for MyWS {
    type State = ();

    fn request(req: HttpRequest,
               payload: Option<Payload>,
               ctx: &mut HttpContext<Self>) -> Reply<Self>
    {
        if let Some(payload) = payload {
            match ws::handshake(req) {
                Ok(resp) => {
                    ctx.start(resp);
                    ctx.add_stream(ws::WsStream::new(payload));
                    Reply::stream(MyWS{})
                },
                Err(err) =>
                    Reply::reply(err)
            }
        } else {
            Reply::with(req, httpcodes::HTTPBadRequest)
        }
    }
}

impl ResponseType<ws::Message> for MyWS {
    type Item = ();
    type Error = ();
}

impl StreamHandler<ws::Message> for MyWS {}

impl Handler<ws::Message> for MyWS {
    fn handle(&mut self, msg: ws::Message, ctx: &mut HttpContext<Self>)
              -> Response<Self, ws::Message>
    {
        println!("WS: {:?}", msg);
        match msg {
            ws::Message::Ping(msg) => ws::WsWriter::pong(ctx, msg),
            ws::Message::Text(text) => ws::WsWriter::text(ctx, text),
            ws::Message::Binary(bin) => ws::WsWriter::binary(ctx, bin),
            _ => (),
        }
        Self::empty()
    }
}


fn main() {
    let _ = env_logger::init();

    let sys = actix::System::new("http-example");

    let mut routes = RoutingMap::default();

    let mut app = Application::default();
    app.add("/test")
        .get::<MyRoute>()
        .post::<MyRoute>();

    routes.add("/blah", app);

    routes.add_resource("/test")
        .post::<MyRoute>();

    routes.add_resource("/ws/")
        .get::<MyWS>();

    let http = HttpServer::new(routes);
    http.serve::<()>(
        &net::SocketAddr::from_str("127.0.0.1:9080").unwrap()).unwrap();

    println!("starting");
    let _ = sys.run();
}
