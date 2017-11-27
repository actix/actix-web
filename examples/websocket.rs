//! Simple echo websocket server.
//! Open `http://localhost:8080/ws/index.html` in browser
//! or [python console client](https://github.com/actix/actix-web/blob/master/examples/websocket-client.py)
//! could be used for testing.

#![allow(unused_variables)]
extern crate actix;
extern crate actix_web;
extern crate env_logger;

use actix::*;
use actix_web::*;


struct MyWebSocket;

impl Actor for MyWebSocket {
    type Context = HttpContext<Self>;
}

/// Http route handler
impl Route for MyWebSocket {
    type State = ();

    fn request(mut req: HttpRequest, ctx: &mut HttpContext<Self>) -> RouteResult<Self>
    {
        // websocket handshake
        let resp = ws::handshake(&req)?;
        // send HttpResponse back to peer
        ctx.start(resp);
        // convert bytes stream to a stream of `ws::Message` and register it
        ctx.add_stream(ws::WsStream::new(&mut req));
        Reply::async(MyWebSocket)
    }
}

/// Standard actix's stream handler for a stream of `ws::Message`
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
        // process websocket messages
        println!("WS: {:?}", msg);
        match msg {
            ws::Message::Ping(msg) => ws::WsWriter::pong(ctx, &msg),
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
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    let _ = env_logger::init();
    let sys = actix::System::new("ws-example");

    HttpServer::new(
        Application::default("/")
            // enable logger
            .middleware(middlewares::Logger::default())
            // websocket route
            .resource("/ws/", |r| r.get::<MyWebSocket>())
            .route_handler("/", StaticFiles::new("examples/static/", true)))
        // start http server on 127.0.0.1:8080
        .serve::<_, ()>("127.0.0.1:8080").unwrap();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
