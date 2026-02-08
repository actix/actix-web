use actix_multipart::form::{text::Text, MultipartForm};

#[derive(MultipartForm)]
struct Form {
    #[multipart(limit = "2 bytes")]
    description: Text<String>,
}

#[derive(MultipartForm)]
struct Form2 {
    #[multipart(limit = "2 megabytes")]
    description: Text<String>,
}

#[derive(MultipartForm)]
struct Form3 {
    #[multipart(limit = "four meters")]
    description: Text<String>,
}

fn main() {}
