#[macro_use] extern crate actix;
extern crate bytes;
extern crate byteorder;
extern crate futures;
extern crate tokio_io;
extern crate tokio_core;
extern crate serde;
extern crate serde_json;
#[macro_use] extern crate serde_derive;

use std::{io, net, process, thread};
use std::str::FromStr;
use std::time::Duration;
use futures::Future;
use tokio_io::AsyncRead;
use tokio_io::io::WriteHalf;
use tokio_io::codec::FramedRead;
use tokio_core::net::TcpStream;
use actix::prelude::*;

mod codec;


fn main() {
    let sys = actix::System::new("chat-client");

    // Connect to server
    let addr = net::SocketAddr::from_str("127.0.0.1:12345").unwrap();
    Arbiter::handle().spawn(
        TcpStream::connect(&addr, Arbiter::handle())
            .and_then(|stream| {
                let addr: Addr<Syn, _> = ChatClient::create(|ctx| {
                    let (r, w) = stream.split();
                    ChatClient::add_stream(FramedRead::new(r, codec::ClientChatCodec), ctx);
                    ChatClient{
                        framed: actix::io::FramedWrite::new(
                            w, codec::ClientChatCodec, ctx)}});

                // start console loop
                thread::spawn(move|| {
                    loop {
                        let mut cmd = String::new();
                        if io::stdin().read_line(&mut cmd).is_err() {
                            println!("error");
                            return
                        }

                        addr.do_send(ClientCommand(cmd));
                    }
                });

                futures::future::ok(())
            })
            .map_err(|e| {
                println!("Can not connect to server: {}", e);
                process::exit(1)
            })
    );

    println!("Running chat client");
    sys.run();
}


struct ChatClient {
    framed: actix::io::FramedWrite<WriteHalf<TcpStream>, codec::ClientChatCodec>,
}

#[derive(Message)]
struct ClientCommand(String);

impl Actor for ChatClient {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Context<Self>) {
        // start heartbeats otherwise server will disconnect after 10 seconds
        self.hb(ctx)
    }

    fn stopped(&mut self, _: &mut Context<Self>) {
        println!("Disconnected");

        // Stop application on disconnect
        Arbiter::system().do_send(actix::msgs::SystemExit(0));
    }
}

impl ChatClient {
    fn hb(&self, ctx: &mut Context<Self>) {
        ctx.run_later(Duration::new(1, 0), |act, ctx| {
            act.framed.write(codec::ChatRequest::Ping);
            act.hb(ctx);
        });
    }
}

impl actix::io::WriteHandler<io::Error> for ChatClient {}

/// Handle stdin commands
impl Handler<ClientCommand> for ChatClient {
    type Result = ();

    fn handle(&mut self, msg: ClientCommand, _: &mut Context<Self>) {
        let m = msg.0.trim();
        if m.is_empty() {
            return
        }

        // we check for /sss type of messages
        if m.starts_with('/') {
            let v: Vec<&str> = m.splitn(2, ' ').collect();
            match v[0] {
                "/list" => {
                    self.framed.write(codec::ChatRequest::List);
                },
                "/join" => {
                    if v.len() == 2 {
                        self.framed.write(codec::ChatRequest::Join(v[1].to_owned()));
                    } else {
                        println!("!!! room name is required");
                    }
                },
                _ => println!("!!! unknown command"),
            }
        } else {
            self.framed.write(codec::ChatRequest::Message(m.to_owned()));
        }
    }
}

/// Server communication

impl StreamHandler<codec::ChatResponse, io::Error> for ChatClient {

    fn handle(&mut self, msg: codec::ChatResponse, _: &mut Context<Self>) {
        match msg {
            codec::ChatResponse::Message(ref msg) => {
                println!("message: {}", msg);
            }
            codec::ChatResponse::Joined(ref msg) => {
                println!("!!! joined: {}", msg);
            }
            codec::ChatResponse::Rooms(rooms) => {
                println!("\n!!! Available rooms:");
                for room in rooms {
                    println!("{}", room);
                }
                println!("");
            }
            _ => (),
        }
    }
}
