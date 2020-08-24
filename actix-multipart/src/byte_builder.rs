use bytes::{Bytes, BytesMut};

pub trait FromBytes {
    fn from_bytes(next: Bytes) -> Self;
}

impl FromBytes for String {
    fn from_bytes(bytes: Bytes) -> Self {
        std::str::from_utf8(&bytes)
            .expect("string field is not utf-8")
            .to_owned()
    }
}

impl FromBytes for Bytes {
    fn from_bytes(bytes: Bytes) -> Self {
        bytes
    }
}

impl FromBytes for BytesMut {
    fn from_bytes(bytes: Bytes) -> Self {
        BytesMut::from(bytes.as_ref())
    }
}
