use actix_multipart_derive::MultipartForm;
use bytes::BytesMut;

#[derive(Debug, Clone, Default, MultipartForm)]
struct Form {
    name: String,
    #[multipart(max_size = 8096)]
    file: BytesMut,
}

fn main () {}
