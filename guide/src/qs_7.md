# HttpRequest & HttpResponse

## Content encoding

Actix automatically *compress*/*decompress* payload. 
Following codecs are supported: 

 * Brotli
 * Gzip
 * Deflate
 * Identity
 
 If request headers contains `Content-Encoding` header, request payload get decompressed
 according to header value. Multiple codecs are not supported, i.e: `Content-Encoding: br, gzip`.
 
Response payload get compressed based on *content_encoding* parameter. 
By default `ContentEncoding::Auto` is used. If `ContentEncoding::Auto` is selected
then compression depends on request's `Accept-Encoding` header. 
`ContentEncoding::Identity` could be used to disable compression.
If other content encoding is selected the compression is enforced for this codec. For example,
to enable `brotli` response's body compression use `ContentEncoding::Br`:

```rust
extern crate actix_web;
use actix_web::*;

fn index(req: HttpRequest) -> HttpResponse {
    HttpResponse::Ok()
        .content_encoding(ContentEncoding::Br)
        .body("data").unwrap()
}
# fn main() {}
```
 
## JSON Response

The `Json` type allows you to respond with well-formed JSON data: simply return a value of 
type Json<T> where T is the type of a structure to serialize into *JSON*. The 
type `T` must implement the `Serialize` trait from *serde*.

```rust
extern crate actix_web;
#[macro_use] extern crate serde_derive;
use actix_web::*;

#[derive(Serialize)]
struct MyObj {
    name: String,
}

fn index(req: HttpRequest) -> Result<Json<MyObj>> {
    Ok(Json(MyObj{name: req.match_info().query("name")?}))
}

fn main() {
    Application::default("/")
        .resource(r"/a/{name}", |r| r.method(Method::GET).f(index))
        .finish();
}
```
