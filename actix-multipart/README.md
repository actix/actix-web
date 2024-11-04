# `actix-multipart`

<!-- prettier-ignore-start -->

[![crates.io](https://img.shields.io/crates/v/actix-multipart?label=latest)](https://crates.io/crates/actix-multipart)
[![Documentation](https://docs.rs/actix-multipart/badge.svg?version=0.7.2)](https://docs.rs/actix-multipart/0.7.2)
![Version](https://img.shields.io/badge/rustc-1.72+-ab6000.svg)
![MIT or Apache 2.0 licensed](https://img.shields.io/crates/l/actix-multipart.svg)
<br />
[![dependency status](https://deps.rs/crate/actix-multipart/0.7.2/status.svg)](https://deps.rs/crate/actix-multipart/0.7.2)
[![Download](https://img.shields.io/crates/d/actix-multipart.svg)](https://crates.io/crates/actix-multipart)
[![Chat on Discord](https://img.shields.io/discord/771444961383153695?label=chat&logo=discord)](https://discord.gg/NWpN5mmg3x)

<!-- prettier-ignore-end -->

<!-- cargo-rdme start -->

Multipart request & form support for Actix Web.

The [`Multipart`] extractor aims to support all kinds of `multipart/*` requests, including `multipart/form-data`, `multipart/related` and `multipart/mixed`. This is a lower-level extractor which supports reading [multipart fields](Field), in the order they are sent by the client.

Due to additional requirements for `multipart/form-data` requests, the higher level [`MultipartForm`] extractor and derive macro only supports this media type.

## Examples

```rust
use actix_web::{post, App, HttpServer, Responder};

use actix_multipart::form::{json::Json as MpJson, tempfile::TempFile, MultipartForm};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Metadata {
    name: String,
}

#[derive(Debug, MultipartForm)]
struct UploadForm {
    #[multipart(limit = "100MB")]
    file: TempFile,
    json: MpJson<Metadata>,
}

#[post("/videos")]
pub async fn post_video(MultipartForm(form): MultipartForm<UploadForm>) -> impl Responder {
    format!(
        "Uploaded file {}, with size: {}",
        form.json.name, form.file.size
    )
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(move || App::new().service(post_video))
        .bind(("127.0.0.1", 8080))?
        .run()
        .await
}
```

cURL request:

```sh
curl -v --request POST \
  --url http://localhost:8080/videos \
  -F 'json={"name": "Cargo.lock"};type=application/json' \
  -F file=@./Cargo.lock
```

[`MultipartForm`]: struct@form::MultipartForm

<!-- cargo-rdme end -->

[More available in the examples repo &rarr;](https://github.com/actix/examples/tree/master/forms/multipart)
