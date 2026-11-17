use actix_multipart::form::{bytes::Bytes, MultipartForm};

#[derive(MultipartForm)]
#[multipart(max_parts = -1)]
struct Negative {}

#[derive(MultipartForm)]
#[multipart(max_parts = "many")]
struct InvalidString {}

#[derive(MultipartForm)]
#[multipart(max_parts = 1.5)]
struct Fractional {}

#[derive(MultipartForm)]
#[multipart(max_parts = 1, max_parts = 2)]
struct Duplicate {}

#[derive(MultipartForm)]
struct FieldAttribute {
    #[multipart(max_parts = 2)]
    files: Vec<Bytes>,
}

fn main() {}
