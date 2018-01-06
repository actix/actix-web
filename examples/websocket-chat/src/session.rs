//! `ClientSession` is an actor, it manages peer tcp connection and
//! proxies commands from peer to `ChatServer`.
use std::{io, net};
use std::str::FromStr;
use std::time::{Instant, Duration};
use futures::Stream;
use tokio_core::net::{TcpStream, TcpListener};

use actix::prelude::*;

use server::{self, ChatServer};
use codec::{ChatRequest, ChatResponse, ChatCodec};


/// Chat server sends this messages to session
#[derive(Message)]
pub struct Message(pub String);

/// `ChatSession` actor is responsible for tcp peer communitions.
pub struct ChatSession {
    /// unique session id
    id: usize,
    /// this is address of chat server
    addr: SyncAddress<ChatServer>,
    /// Client must send ping at least once per 10 seconds, otherwise we drop connection.
    hb: Instant,
    /// joined room
    room: String,
}

impl Actor for ChatSession {
    /// For tcp communication we are going to use `FramedContext`.
    /// It is convinient wrapper around `Framed` object from `tokio_io`
    type Context = FramedContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        // we'll start heartbeat process on session start.
        self.hb(ctx);

        // register self in chat server. `AsyncContext::wait` register
        // future within context, but context waits until this future resolves
        // before processing any other events.
        let addr: SyncAddress<_> = ctx.address();
        self.addr.call(self, server::Connect{addr: addr.subscriber()})
            .then(|res, act, ctx| {
                match res {
                    Ok(Ok(res)) => act.id = res,
                    // something is wrong with chat server
                    _ => ctx.stop(),
                }
                actix::fut::ok(())
            }).wait(ctx);
    }

    fn stopping(&mut self, ctx: &mut Self::Context) {
        // notify chat server
        self.addr.send(server::Disconnect{id: self.id});
        ctx.stop()
    }
}

/// To use `FramedContext` we have to define Io type and Codec
impl FramedActor for ChatSession {
    type Io = TcpStream;
    type Codec= ChatCodec;

    /// This is main event loop for client requests
    fn handle(&mut self, msg: io::Result<ChatRequest>, ctx: &mut FramedContext<Self>) {
        match msg {
            Err(_) => ctx.stop(),
            Ok(msg) => match msg {
                ChatRequest::List => {
                    // Send ListRooms message to chat server and wait for response
                    println!("List rooms");
                    self.addr.call(self, server::ListRooms).then(|res, _, ctx| {
                        match res {
                            Ok(Ok(rooms)) => {
                                let _ = ctx.send(ChatResponse::Rooms(rooms));
                            },
                        _ => println!("Something is wrong"),
                        }
                        actix::fut::ok(())
                    }).wait(ctx)
                    // .wait(ctx) pauses all events in context,
                    // so actor wont receive any new messages until it get list of rooms back
                },
                ChatRequest::Join(name) => {
                    println!("Join to room: {}", name);
                    self.room = name.clone();
                    self.addr.send(server::Join{id: self.id, name: name.clone()});
                    let _ = ctx.send(ChatResponse::Joined(name));
                },
                ChatRequest::Message(message) => {
                    // send message to chat server
                    println!("Peer message: {}", message);
                    self.addr.send(
                        server::Message{id: self.id,
                                        msg: message, room:
                                        self.room.clone()})
                }
                // we update heartbeat time on ping from peer
                ChatRequest::Ping =>
                    self.hb = Instant::now(),
            }
        }
    }
}

/// Handler for Message, chat server sends this message, we just send string to peer
impl Handler<Message> for ChatSession {
    type Result = ();

    fn handle(&mut self, msg: Message, ctx: &mut FramedContext<Self>) {
        // send message to peer
        let _ = ctx.send(ChatResponse::Message(msg.0));
    }
}

/// Helper methods
impl ChatSession {

    pub fn new(addr: SyncAddress<ChatServer>) -> ChatSession {
        ChatSession {id: 0, addr: addr, hb: Instant::now(), room: "Main".to_owned()}
    }
    
    /// helper method that sends ping to client every second.
    ///
    /// also this method check heartbeats from client
    fn hb(&self, ctx: &mut FramedContext<Self>) {
        ctx.run_later(Duration::new(1, 0), |act, ctx| {
            // check client heartbeats
            if Instant::now().duration_since(act.hb) > Duration::new(10, 0) {
                // heartbeat timed out
                println!("Client heartbeat failed, disconnecting!");

                // notify chat server
                act.addr.send(server::Disconnect{id: act.id});

                // stop actor
                ctx.stop();
            }

            if ctx.send(ChatResponse::Ping).is_ok() {
                // if we can not send message to sink, sink is closed (disconnected)
                act.hb(ctx);
            }
        });
    }
}


/// Define tcp server that will accept incomint tcp connection and create
/// chat actors.
pub struct TcpServer {
    chat: SyncAddress<ChatServer>,
}

impl TcpServer {
    pub fn new(s: &str, chat: SyncAddress<ChatServer>) {
        // Create server listener
        let addr = net::SocketAddr::from_str("127.0.0.1:12345").unwrap();
        let listener = TcpListener::bind(&addr, Arbiter::handle()).unwrap();

        // Our chat server `Server` is an actor, first we need to start it
        // and then add stream on incoming tcp connections to it.
        // TcpListener::incoming() returns stream of the (TcpStream, net::SocketAddr) items
        // So to be able to handle this events `Server` actor has to implement
        // stream handler `StreamHandler<(TcpStream, net::SocketAddr), io::Error>`
        let _: () = TcpServer::create(|ctx| {
            ctx.add_message_stream(listener.incoming()
                                   .map_err(|_| ())
                                   .map(|(t, a)| TcpConnect(t, a)));
            TcpServer{chat: chat}
        });
    }
}

/// Make actor from `Server`
impl Actor for TcpServer {
    /// Every actor has to provide execution `Context` in which it can run.
    type Context = Context<Self>;
}

#[derive(Message)]
struct TcpConnect(TcpStream, net::SocketAddr);

/// Handle stream of TcpStream's
impl Handler<TcpConnect> for TcpServer {
    type Result = ();

    fn handle(&mut self, msg: TcpConnect, _: &mut Context<Self>) {
        // For each incoming connection we create `ChatSession` actor
        // with out chat server address.
        let server = self.chat.clone();
        let _: () = ChatSession::new(server).framed(msg.0, ChatCodec);
    }
}
