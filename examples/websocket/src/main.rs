#![allow(unused_variables)]
extern crate actix;
extern crate actix_web;

use actix::*;
use actix_web::*;


struct MyWebSocket;

impl Actor for MyWebSocket {
    type Context = HttpContext<Self>;
}

impl Route for MyWebSocket {
    type State = ();

    fn request(req: HttpRequest, payload: Payload, ctx: &mut HttpContext<Self>) -> Reply<Self>
    {
        match ws::handshake(&req) {
            Ok(resp) => {
                ctx.start(resp);
                ctx.add_stream(ws::WsStream::new(payload));
                Reply::async(MyWebSocket)
            }
            Err(err) => {
                Reply::reply(err)
            }
        }
    }
}

impl StreamHandler<ws::Message> for MyWebSocket {
    fn started(&mut self, ctx: &mut Self::Context) {
        println!("WebSocket session openned");
    }

    fn finished(&mut self, ctx: &mut Self::Context) {
        println!("WebSocket session closed");
    }
}

impl Handler<ws::Message> for MyWebSocket {
    fn handle(&mut self, msg: ws::Message, ctx: &mut HttpContext<Self>)
              -> Response<Self, ws::Message>
    {
        println!("WS: {:?}", msg);
        match msg {
            ws::Message::Ping(msg) => ws::WsWriter::pong(ctx, msg),
            ws::Message::Text(text) => ws::WsWriter::text(ctx, &text),
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
    let sys = actix::System::new("ws-example");

    HttpServer::new(
        RoutingMap::default()
            .resource("/ws/", |r| r.get::<MyWebSocket>())
            .app("/", Application::default()
                 .route_handler("/", StaticFiles::new("static/", true))
                 .finish())
            .finish())
        .serve::<_, ()>("127.0.0.1:8080").unwrap();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
