//! `ChatServer` is an actor. It maintains list of connection client session.
//! And manages available rooms. Peers send messages to other peers in same
//! room through `ChatServer`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use rand::{self, Rng, ThreadRng};
use actix::prelude::*;

use session;

/// Message for chat server communications

/// New chat session is created
pub struct Connect {
    pub addr: Box<Subscriber<session::Message> + Send>,
}

/// Session is disconnected
pub struct Disconnect {
    pub id: usize,
}

/// Send message to specific room
pub struct Message {
    /// Id of the client session
    pub id: usize,
    /// Peer message
    pub msg: String,
    /// Room name
    pub room: String,
}

/// List of available rooms
pub struct ListRooms;

/// Join room, if room does not exists create new one.
pub struct Join {
    /// Client id
    pub id: usize,
    /// Room name
    pub name: String,
}

/// `ChatServer` manages chat rooms and responsible for coordinating chat session.
/// implementation is super primitive
pub struct ChatServer {
    sessions: HashMap<usize, Box<Subscriber<session::Message> + Send>>,
    rooms: HashMap<String, HashSet<usize>>,
    rng: RefCell<ThreadRng>,
}

impl Default for ChatServer {
    fn default() -> ChatServer {
        // default room
        let mut rooms = HashMap::new();
        rooms.insert("Main".to_owned(), HashSet::new());

        ChatServer {
            sessions: HashMap::new(),
            rooms: rooms,
            rng: RefCell::new(rand::thread_rng()),
        }
    }
}

impl ChatServer {
    /// Send message to all users in the room
    fn send_message(&self, room: &str, message: &str, skip_id: usize) {
        if let Some(sessions) = self.rooms.get(room) {
            for id in sessions {
                if *id != skip_id {
                    if let Some(addr) = self.sessions.get(id) {
                        let _ = addr.send(session::Message(message.to_owned()));
                    }
                }
            }
        }
    }
}

/// Make actor from `ChatServer`
impl Actor for ChatServer {
    /// We are going to use simple Context, we just need ability to communicate
    /// with other actors.
    type Context = Context<Self>;
}

/// Handler for Connect message.
///
/// Register new session and assign unique id to this session
impl Handler<Connect> for ChatServer {

    fn handle(&mut self, msg: Connect, _: &mut Context<Self>) -> Response<Self, Connect> {
        println!("Someone joined");

        // notify all users in same room
        self.send_message(&"Main".to_owned(), "Someone joined", 0);

        // register session with random id
        let id = self.rng.borrow_mut().gen::<usize>();
        self.sessions.insert(id, msg.addr);

        // auto join session to Main room
        self.rooms.get_mut(&"Main".to_owned()).unwrap().insert(id);

        // send id back
        Self::reply(id)
    }
}

impl ResponseType<Connect> for ChatServer {
    /// Response type for Connect message
    ///
    /// Chat server returns unique session id
    type Item = usize;
    type Error = ();
}


/// Handler for Disconnect message.
impl Handler<Disconnect> for ChatServer {

    fn handle(&mut self, msg: Disconnect, _: &mut Context<Self>) -> Response<Self, Disconnect> {
        println!("Someone disconnected");

        let mut rooms: Vec<String> = Vec::new();

        // remove address
        if self.sessions.remove(&msg.id).is_some() {
            // remove session from all rooms
            for (name, sessions) in &mut self.rooms {
                if sessions.remove(&msg.id) {
                    rooms.push(name.to_owned());
                }
            }
        }
        // send message to other users
        for room in rooms {
            self.send_message(&room, "Someone disconnected", 0);
        }

        Self::empty()
    }
}

impl ResponseType<Disconnect> for ChatServer {
    type Item = ();
    type Error = ();
}

/// Handler for Message message.
impl Handler<Message> for ChatServer {

    fn handle(&mut self, msg: Message, _: &mut Context<Self>) -> Response<Self, Message> {
        self.send_message(&msg.room, msg.msg.as_str(), msg.id);

        Self::empty()
    }
}

impl ResponseType<Message> for ChatServer {
    type Item = ();
    type Error = ();
}

/// Handler for `ListRooms` message.
impl Handler<ListRooms> for ChatServer {

    fn handle(&mut self, _: ListRooms, _: &mut Context<Self>) -> Response<Self, ListRooms> {
        let mut rooms = Vec::new();

        for key in self.rooms.keys() {
            rooms.push(key.to_owned())
        }

        Self::reply(rooms)
    }
}

impl ResponseType<ListRooms> for ChatServer {
    type Item = Vec<String>;
    type Error = ();
}

/// Join room, send disconnect message to old room
/// send join message to new room
impl Handler<Join> for ChatServer {

    fn handle(&mut self, msg: Join, _: &mut Context<Self>) -> Response<Self, Join> {
        let Join {id, name} = msg;
        let mut rooms = Vec::new();

        // remove session from all rooms
        for (n, sessions) in &mut self.rooms {
            if sessions.remove(&id) {
                rooms.push(n.to_owned());
            }
        }
        // send message to other users
        for room in rooms {
            self.send_message(&room, "Someone disconnected", 0);
        }

        if self.rooms.get_mut(&name).is_none() {
            self.rooms.insert(name.clone(), HashSet::new());
        }
        self.send_message(&name, "Someone connected", id);
        self.rooms.get_mut(&name).unwrap().insert(id);

        Self::empty()
    }
}

impl ResponseType<Join> for ChatServer {
    type Item = ();
    type Error = ();
}
