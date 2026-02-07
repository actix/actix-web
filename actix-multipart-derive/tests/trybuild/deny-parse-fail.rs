use actix_multipart::form::MultipartForm;

#[derive(MultipartForm)]
#[multipart(duplicate_field = "no")]
struct Form {}

fn main() {}
