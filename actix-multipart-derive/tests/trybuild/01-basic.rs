use actix_multipart_derive::MultipartForm;
use bytes::BytesMut;

#[derive(Debug, Clone, Default, MultipartForm)]
struct Form {
    name: String,
    file: BytesMut,
}

fn main() {}
