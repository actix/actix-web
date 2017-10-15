// #![feature(try_trait)]
#![allow(dead_code, unused_variables)]
extern crate actix;
extern crate actix_web;
extern crate tokio_core;
extern crate env_logger;

use actix::*;
use actix_web::*;

struct MyRoute {req: Option<HttpRequest>}

impl Actor for MyRoute {
    type Context = HttpContext<Self>;
}

impl Route for MyRoute {
    type State = ();

    fn request(req: HttpRequest, payload: Payload, ctx: &mut HttpContext<Self>) -> Reply<Self> {
        if !payload.eof() {
            ctx.add_stream(payload);
            Reply::stream(MyRoute{req: Some(req)})
        } else {
            Reply::reply(httpcodes::HTTPOk)
        }
    }
}

impl ResponseType<PayloadItem> for MyRoute {
    type Item = ();
    type Error = ();
}

impl StreamHandler<PayloadItem> for MyRoute {}

impl Handler<PayloadItem> for MyRoute {
    fn handle(&mut self, msg: PayloadItem, ctx: &mut HttpContext<Self>)
              -> Response<Self, PayloadItem>
    {
        println!("CHUNK: {:?}", msg);
        if let Some(req) = self.req.take() {
            ctx.start(httpcodes::HTTPOk);
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

    fn request(req: HttpRequest, payload: Payload, ctx: &mut HttpContext<Self>) -> Reply<Self>
    {
        match ws::handshake(&req) {
            Ok(resp) => {
                ctx.start(resp);
                ctx.add_stream(ws::WsStream::new(payload));
                Reply::stream(MyWS{})
            }
            Err(err) => {
                Reply::reply(err)
            }
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
            ws::Message::Closed | ws::Message::Error => {
                ctx.stop();
            }
            _ => (),
        }
        Self::empty()
    }
}


fn main() {
    let _ = env_logger::init();

    let sys = actix::System::new("http-example");

    HttpServer::new(
        RoutingMap::default()
            .app("/blah", Application::default()
                 .resource("/test", |r| {
                     r.get::<MyRoute>();
                     r.post::<MyRoute>();
                 })
                 .finish())
            .resource("/test", |r| r.post::<MyRoute>())
            .resource("/ws/", |r| r.get::<MyWS>())
            .resource("/simple/", |r|
                      r.handler(Method::GET, |req, payload, state| {
                          httpcodes::HTTPOk
                      }))
            .finish())
        .serve::<_, ()>("127.0.0.1:9080").unwrap();

    println!("starting");
    let _ = sys.run();
}
