use actix_multipart::form::{bytes::Bytes, MultipartCollect, MultipartForm};
use actix_web::{web, App};

#[derive(MultipartForm)]
#[multipart(max_parts = 10, deny_unknown_fields, duplicate_field = "deny")]
struct Upload {
    #[multipart(limit = "2 KiB")]
    files: Vec<Bytes>,
}

#[derive(MultipartForm)]
#[multipart(max_parts = 0)]
struct Empty {}

#[derive(MultipartForm)]
struct DefaultForm {}

fn main() {
    assert_eq!(Upload::max_parts(), Some(10));
    assert_eq!(Empty::max_parts(), Some(0));
    assert_eq!(DefaultForm::max_parts(), None);

    App::new().default_service(web::to(|_: MultipartForm<Upload>| async { "ok" }));
}
