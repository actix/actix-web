use std::net;
use std::rc::Rc;
use std::cell::{Cell, RefCell, RefMut};

use helpers;
use super::channel::Node;
use super::shared::{SharedBytes, SharedBytesPool};

/// Various server settings
#[derive(Debug, Clone)]
pub struct ServerSettings {
    addr: Option<net::SocketAddr>,
    secure: bool,
    host: String,
}

impl Default for ServerSettings {
    fn default() -> Self {
        ServerSettings {
            addr: None,
            secure: false,
            host: "localhost:8080".to_owned(),
        }
    }
}

impl ServerSettings {
    /// Crate server settings instance
    pub(crate) fn new(addr: Option<net::SocketAddr>, host: &Option<String>, secure: bool)
                      -> ServerSettings
    {
        let host = if let Some(ref host) = *host {
            host.clone()
        } else if let Some(ref addr) = addr {
            format!("{}", addr)
        } else {
            "localhost".to_owned()
        };
        ServerSettings { addr, secure, host }
    }

    /// Returns the socket address of the local half of this TCP connection
    pub fn local_addr(&self) -> Option<net::SocketAddr> {
        self.addr
    }

    /// Returns true if connection is secure(https)
    pub fn secure(&self) -> bool {
        self.secure
    }

    /// Returns host header value
    pub fn host(&self) -> &str {
        &self.host
    }
}


pub(crate) struct WorkerSettings<H> {
    h: RefCell<Vec<H>>,
    enabled: bool,
    keep_alive: u64,
    bytes: Rc<SharedBytesPool>,
    messages: Rc<helpers::SharedMessagePool>,
    channels: Cell<usize>,
    node: Box<Node<()>>,
}

impl<H> WorkerSettings<H> {
    pub(crate) fn new(h: Vec<H>, keep_alive: Option<u64>) -> WorkerSettings<H> {
        WorkerSettings {
            h: RefCell::new(h),
            enabled: if let Some(ka) = keep_alive { ka > 0 } else { false },
            keep_alive: keep_alive.unwrap_or(0),
            bytes: Rc::new(SharedBytesPool::new()),
            messages: Rc::new(helpers::SharedMessagePool::new()),
            channels: Cell::new(0),
            node: Box::new(Node::head()),
        }
    }

    pub fn num_channels(&self) -> usize {
        self.channels.get()
    }

    pub fn head(&self) -> &Node<()> {
        &self.node
    }

    pub fn handlers(&self) -> RefMut<Vec<H>> {
        self.h.borrow_mut()
    }

    pub fn keep_alive(&self) -> u64 {
        self.keep_alive
    }

    pub fn keep_alive_enabled(&self) -> bool {
        self.enabled
    }

    pub fn get_shared_bytes(&self) -> SharedBytes {
        SharedBytes::new(self.bytes.get_bytes(), Rc::clone(&self.bytes))
    }

    pub fn get_http_message(&self) -> helpers::SharedHttpInnerMessage {
        helpers::SharedHttpInnerMessage::new(self.messages.get(), Rc::clone(&self.messages))
    }

    pub fn add_channel(&self) {
        self.channels.set(self.channels.get() + 1);
    }

    pub fn remove_channel(&self) {
        let num = self.channels.get();
        if num > 0 {
            self.channels.set(num-1);
        } else {
            error!("Number of removed channels is bigger than added channel. Bug in actix-web");
        }
    }
}
