use bytes::{BufMut, Bytes, BytesMut};

pub trait BuildFromBytes {
    fn append(&mut self, next: Bytes);
}

impl BuildFromBytes for String {
    fn append(&mut self, chunk: Bytes) {
        let chunk_str = std::str::from_utf8(&chunk).expect("string field is not utf-8");
        self.push_str(chunk_str);
    }
}

impl BuildFromBytes for BytesMut {
    fn append(&mut self, chunk: Bytes) {
        self.put(&chunk[..]);
    }
}
