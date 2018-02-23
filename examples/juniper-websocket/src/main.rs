//! Simple echo websocket server.
//! Open `http://localhost:8080/ws/index.html` in browser
//! or [python console client](https://github.com/actix/actix-web/blob/master/examples/websocket-client.py)
//! could be used for testing.

#![allow(unused_variables)]
extern crate actix;
extern crate actix_web;
extern crate env_logger;
#[macro_use]
extern crate juniper;
#[macro_use]
extern crate lazy_static;
extern crate serde_json;

use actix::*;
use actix_web::*;
use juniper::http::GraphQLRequest;

mod schema;

use schema::Schema;
use schema::create_schema;

lazy_static! {
    static ref SCHEMA: Schema = create_schema();
}

fn graphiql(_req: HttpRequest) -> Result<HttpResponse> {
    let html = r#"
        <!DOCTYPE html>
        <html>
        <head>
            <title>GraphQL</title>
            <style>
                html, body, #app {
                    height: 100%;
                    margin: 0;
                    overflow: hidden;
                    width: 100%;
                }
            </style>
            <link rel="stylesheet" type="text/css" href="//cdnjs.cloudflare.com/ajax/libs/graphiql/0.10.2/graphiql.css">
        </head>
        <body>
            <div id="app"></div>
            <script src="//cdnjs.cloudflare.com/ajax/libs/fetch/2.0.3/fetch.js"></script>
            <script src="//cdnjs.cloudflare.com/ajax/libs/react/16.2.0/umd/react.production.min.js"></script>
            <script src="//cdnjs.cloudflare.com/ajax/libs/react-dom/16.2.0/umd/react-dom.production.min.js"></script>
            <script src="//cdnjs.cloudflare.com/ajax/libs/graphiql/0.11.11/graphiql.min.js"></script>
            <script>
                var wsUri = (window.location.protocol=='https:'&&'wss://'||'ws://')+window.location.host + '/ws/';
                promiseResolver = null;
                conn = new WebSocket(wsUri);
                console.log('Connecting...');
                conn.onopen = function() {
                    console.log('Connected.');
                    ReactDOM.render(React.createElement(GraphiQL, { fetcher: graphQLFetcher }), document.querySelector('#app'));
                };
                conn.onmessage = function(e) {
                    console.log('Received: ' + e.data);
                    if (promiseResolver) {
                        try {
                            promiseResolver(JSON.parse(e.data));
                        } catch (error) {
                            promiseResolver(e.data);
                        }
                    }
                };
                conn.onclose = function() {
                    console.log('Disconnected.');
                    conn = null;
                };
                function graphQLFetcher(params) {
                    return new Promise((resolve) => {
                        promiseResolver = resolve;
                        conn.send(JSON.stringify(params));
                    })
                }
            </script>
        </body>
        </html>
    "#;
    Ok(HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(html)
        .unwrap())
}

/// do websocket handshake and start `MyWebSocket` actor
fn ws_index(r: HttpRequest) -> Result<HttpResponse> {
    ws::start(r, MyWebSocket)
}

/// websocket connection is long running connection, it easier
/// to handle with an actor
struct MyWebSocket;

impl Actor for MyWebSocket {
    type Context = ws::WebsocketContext<Self>;
}

/// Handler for `ws::Message`
impl Handler<ws::Message> for MyWebSocket {
    type Result = ();

    fn handle(&mut self, msg: ws::Message, ctx: &mut Self::Context) {
        match msg {
            ws::Message::Ping(msg) => ctx.pong(&msg),
            ws::Message::Text(text) => {
                println!("server req: {}", text);
                let req: GraphQLRequest = serde_json::from_str(&text).unwrap();
                let res = req.execute(&SCHEMA, &());
                let res_text = serde_json::to_string(&res).unwrap();
                ctx.text(res_text)
            }
            ws::Message::Binary(bin) => ctx.binary(bin),
            ws::Message::Closed | ws::Message::Error => {
                ctx.stop();
            }
            _ => (),
        }
    }
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    let _ = env_logger::init();
    let sys = actix::System::new("ws-example");
    let schema = create_schema();

    let _addr = HttpServer::new(
        || Application::new()
            // enable logger
            .middleware(middleware::Logger::default())
            // websocket route
            .resource("/ws/", |r| r.method(Method::GET).f(ws_index))
            .resource("/graphiql", |r| r.method(Method::GET).f(graphiql)))
        // start http server on 127.0.0.1:8080
        .bind("127.0.0.1:8080").unwrap()
        .start();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
