# Request & Response

## Response

Builder-like patter is used to construct an instance of `HttpResponse`.
`HttpResponse` provides several method that returns `HttpResponseBuilder` instance,
which is implements various convinience methods that helps build response.
Check [documentation](../actix_web/dev/struct.HttpResponseBuilder.html)
for type description. Methods `.body`, `.finish`, `.json` finalizes response creation,
if this methods get call for the same builder instance, builder will panic.

```rust
# extern crate actix_web;
use actix_web::*;
use actix_web::headers::ContentEncoding;

fn index(req: HttpRequest) -> HttpResponse {
    HttpResponse::Ok()
        .content_encoding(ContentEncoding::Br)
        .content_type("plain/text")
        .header("X-Hdr", "sample")
        .body("data").unwrap()
}
# fn main() {}
```

## Content encoding

Actix automatically *compress*/*decompress* payload. Following codecs are supported: 

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
# extern crate actix_web;
use actix_web::*;
use actix_web::headers::ContentEncoding;

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
# extern crate actix_web;
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
    Application::new()
        .resource(r"/a/{name}", |r| r.method(Method::GET).f(index))
        .finish();
}
```

## Chunked transfer encoding

Actix automatically decode *chunked* encoding. `HttpRequest::payload()` already contains
decoded bytes stream. If request payload compressed with one of supported
compression codecs (br, gzip, deflate) bytes stream get decompressed.

Chunked encoding on response could be enabled with `HttpResponseBuilder::chunked()` method.
But this takes effect only for `Body::Streaming(BodyStream)` or `Body::StreamingContext` bodies.
Also if response payload compression is enabled and streaming body is used, chunked encoding
get enabled automatically.

Enabling chunked encoding for *HTTP/2.0* responses is forbidden.

```rust
# extern crate actix_web;
use actix_web::*;

fn index(req: HttpRequest) -> HttpResponse {
    HttpResponse::Ok()
        .chunked()
        .body(Body::Streaming(payload::Payload::empty().stream())).unwrap()
}
# fn main() {}
```

## Cookies

[WIP]

## Multipart body

Actix provides multipart stream support. 
[*Multipart*](../actix_web/multipart/struct.Multipart.html) is implemented as 
a stream of multipart items, each item could be
[*Field*](../actix_web/multipart/struct.Field.html) or nested *Multipart* stream.
`HttpResponse::multipart()` method returns *Multipart* stream for current request.

In simple form multipart stream handling could be implemented similar to this example

```rust,ignore
# extern crate actix_web;
use actix_web::*;

fn index(req: HttpRequest) -> Box<Future<...>> {
    req.multipart()        // <- get multipart stream for current request
       .and_then(|item| {  // <- iterate over multipart items
           match item {
                           // Handle multipart Field
              multipart::MultipartItem::Field(field) => {
                 println!("==== FIELD ==== {:?} {:?}", field.heders(), field.content_type());
                 
                 Either::A(
                           // Field in turn is a stream of *Bytes* objects
                   field.map(|chunk| {
                        println!("-- CHUNK: \n{}",
                                 std::str::from_utf8(&chunk).unwrap());})
                      .fold((), |_, _| result(Ok(()))))
                },
              multipart::MultipartItem::Nested(mp) => {
                         // Or item could be nested Multipart stream 
                 Either::B(result(Ok(())))
              }
         }
   })
}
```

Full example is available in 
[examples directory](https://github.com/actix/actix-web/tree/master/examples/multipart/).

## Urlencoded body

[WIP]

## Streaming request

Actix uses [*Payload*](../actix_web/payload/struct.Payload.html) object as request payload stream.
*HttpRequest* provides several methods, which can be used for payload access.
At the same time *Payload* implements *Stream* trait, so it could be used with various
stream combinators. Also *Payload* provides serveral convinience methods that return
future object that resolve to Bytes object.

* *readany* method returns *Stream* of *Bytes* objects.

* *readexactly* method returns *Future* that resolves when specified number of bytes
  get received.
  
* *readline* method returns *Future* that resolves when `\n` get received.

* *readuntil* method returns *Future* that resolves when specified bytes string
  matches in input bytes stream

In this example handle reads request payload chunk by chunk and prints every chunk.

```rust
# extern crate actix_web;
# extern crate futures;
# use futures::future::result;
use actix_web::*;
use futures::{Future, Stream};


fn index(mut req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    Box::new(
        req.payload_mut()
            .readany()
            .fold((), |_, chunk| {
                println!("Chunk: {:?}", chunk);
                result::<_, error::PayloadError>(Ok(()))
            })
            .map_err(|e| Error::from(e))
            .map(|_| HttpResponse::Ok().finish().unwrap()))
}
# fn main() {}
```
