mod connect;
mod error;
mod service;

pub use self::connect::Connect;
pub use self::error::ClientError;
pub use self::service::{Client, DefaultClient};

#[derive(PartialEq, Hash, Debug, Clone, Copy)]
pub(crate) enum Protocol {
    Http,
    Https,
    Ws,
    Wss,
}

impl Protocol {
    fn from(s: &str) -> Option<Protocol> {
        match s {
            "http" => Some(Protocol::Http),
            "https" => Some(Protocol::Https),
            "ws" => Some(Protocol::Ws),
            "wss" => Some(Protocol::Wss),
            _ => None,
        }
    }

    fn is_http(self) -> bool {
        match self {
            Protocol::Https | Protocol::Http => true,
            _ => false,
        }
    }

    fn is_secure(self) -> bool {
        match self {
            Protocol::Https | Protocol::Wss => true,
            _ => false,
        }
    }

    fn port(self) -> u16 {
        match self {
            Protocol::Http | Protocol::Ws => 80,
            Protocol::Https | Protocol::Wss => 443,
        }
    }
}
