use actix_multipart_derive::MultipartForm;
use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize, MultipartForm)]
struct Form {
    name: String,

    #[multipart(max_size = 8096)]
    #[serde(rename = "mFile")]
    file: String,
}

fn main() {}
