# Request & Response

## Response

A builder-like pattern is used to construct an instance of `HttpResponse`.
`HttpResponse` provides several methods that return a `HttpResponseBuilder` instance,
which implements various convenience methods that helps building responses.
Check [documentation](../actix_web/dev/struct.HttpResponseBuilder.html)
for type descriptions. The methods `.body`, `.finish`, `.json` finalize response creation and
return a constructed *HttpResponse* instance. If this methods is called for the same
builder instance multiple times, the builder will panic.

```rust
# extern crate actix_web;
use actix_web::{HttpRequest, HttpResponse, http::ContentEncoding};

fn index(req: HttpRequest) -> HttpResponse {
    HttpResponse::Ok()
        .content_encoding(ContentEncoding::Br)
        .content_type("plain/text")
        .header("X-Hdr", "sample")
        .body("data")
}
# fn main() {}
```

## Content encoding

Actix automatically *compresses*/*decompresses* payloads. Following codecs are supported:

 * Brotli
 * Gzip
 * Deflate
 * Identity

 If request headers contain a `Content-Encoding` header, the request payload is decompressed
 according to the header value. Multiple codecs are not supported, i.e: `Content-Encoding: br, gzip`.

Response payload is compressed based on the *content_encoding* parameter.
By default `ContentEncoding::Auto` is used. If `ContentEncoding::Auto` is selected
then compression depends on the request's `Accept-Encoding` header.
`ContentEncoding::Identity` can be used to disable compression.
If another content encoding is selected the compression is enforced for this codec. For example,
to enable `brotli` use `ContentEncoding::Br`:

```rust
# extern crate actix_web;
use actix_web::{HttpRequest, HttpResponse, http::ContentEncoding};

fn index(req: HttpRequest) -> HttpResponse {
    HttpResponse::Ok()
        .content_encoding(ContentEncoding::Br)
        .body("data")
}
# fn main() {}
```


## JSON Request

There are two options for json body deserialization.

The first option is to use *HttpResponse::json()*. This method returns a
[*JsonBody*](../actix_web/dev/struct.JsonBody.html) object which resolves into
the deserialized value.

```rust
# extern crate actix;
# extern crate actix_web;
# extern crate futures;
# extern crate serde_json;
# #[macro_use] extern crate serde_derive;
# use actix_web::*;
# use futures::Future;
#[derive(Debug, Serialize, Deserialize)]
struct MyObj {
    name: String,
    number: i32,
}

fn index(mut req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    req.json().from_err()
        .and_then(|val: MyObj| {
            println!("model: {:?}", val);
            Ok(HttpResponse::Ok().json(val))  // <- send response
        })
        .responder()
}
# fn main() {}
```

Or you can manually load the payload into memory and then deserialize it.
Here is a simple example. We will deserialize a *MyObj* struct. We need to load the request
body first and then deserialize the json into an object.

```rust
# extern crate actix_web;
# extern crate futures;
# use actix_web::*;
# #[macro_use] extern crate serde_derive;
extern crate serde_json;
use futures::{Future, Stream};

#[derive(Serialize, Deserialize)]
struct MyObj {name: String, number: i32}

fn index(req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
   // `concat2` will asynchronously read each chunk of the request body and
   // return a single, concatenated, chunk
   req.concat2()
      // `Future::from_err` acts like `?` in that it coerces the error type from
      // the future into the final error type
      .from_err()
      // `Future::and_then` can be used to merge an asynchronous workflow with a
      // synchronous workflow
      .and_then(|body| {                           // <- body is loaded, now we can deserialize json
          let obj = serde_json::from_slice::<MyObj>(&body)?;
          Ok(HttpResponse::Ok().json(obj))        // <- send response
      })
      .responder()
}
# fn main() {}
```

A complete example for both options is available in
[examples directory](https://github.com/actix/actix-web/tree/master/examples/json/).


## JSON Response

The `Json` type allows you to respond with well-formed JSON data: simply return a value of
type Json<T> where T is the type of a structure to serialize into *JSON*. The
type `T` must implement the `Serialize` trait from *serde*.

```rust
# extern crate actix_web;
#[macro_use] extern crate serde_derive;
use actix_web::{Application, HttpRequest, Json, Result, http::Method};

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

Actix automatically decodes *chunked* encoding. `HttpRequest::payload()` already contains
the decoded byte stream. If the request payload is compressed with one of the supported
compression codecs (br, gzip, deflate) the byte stream is decompressed.

Chunked encoding on response can be enabled with `HttpResponseBuilder::chunked()`.
But this takes effect only for `Body::Streaming(BodyStream)` or `Body::StreamingContext` bodies.
Also if response payload compression is enabled and streaming body is used, chunked encoding
is enabled automatically.

Enabling chunked encoding for *HTTP/2.0* responses is forbidden.

```rust
# extern crate bytes;
# extern crate actix_web;
# extern crate futures;
# use futures::Stream;
use actix_web::*;
use bytes::Bytes;
use futures::stream::once;

fn index(req: HttpRequest) -> HttpResponse {
    HttpResponse::Ok()
        .chunked()
        .body(Body::Streaming(Box::new(once(Ok(Bytes::from_static(b"data"))))))
}
# fn main() {}
```

## Multipart body

Actix provides multipart stream support.
[*Multipart*](../actix_web/multipart/struct.Multipart.html) is implemented as
a stream of multipart items, each item can be a
[*Field*](../actix_web/multipart/struct.Field.html) or a nested *Multipart* stream.
`HttpResponse::multipart()` returns the *Multipart* stream for the current request.

In simple form multipart stream handling can be implemented similar to this example

```rust,ignore
# extern crate actix_web;
use actix_web::*;

fn index(req: HttpRequest) -> Box<Future<...>> {
    req.multipart()        // <- get multipart stream for current request
       .and_then(|item| {  // <- iterate over multipart items
           match item {
                           // Handle multipart Field
              multipart::MultipartItem::Field(field) => {
                 println!("==== FIELD ==== {:?} {:?}", field.headers(), field.content_type());

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

A full example is available in the
[examples directory](https://github.com/actix/actix-web/tree/master/examples/multipart/).

## Urlencoded body

Actix provides support for *application/x-www-form-urlencoded* encoded bodies.
`HttpResponse::urlencoded()` returns a
[*UrlEncoded*](../actix_web/dev/struct.UrlEncoded.html) future, which resolves
into `HashMap<String, String>` which contains decoded parameters.
The *UrlEncoded* future can resolve into a error in several cases:

* content type is not `application/x-www-form-urlencoded`
* transfer encoding is `chunked`.
* content-length is greater than 256k
* payload terminates with error.


```rust
# extern crate actix_web;
# extern crate futures;
use actix_web::*;
use futures::future::{Future, ok};

fn index(mut req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    req.urlencoded()         // <- get UrlEncoded future
       .from_err()
       .and_then(|params| {  // <- url encoded parameters
             println!("==== BODY ==== {:?}", params);
             ok(HttpResponse::Ok().into())
       })
       .responder()
}
# fn main() {}
```


## Streaming request

*HttpRequest* is a stream of `Bytes` objects. It can be used to read the request
body payload.

In this example handle reads the request payload chunk by chunk and prints every chunk.

```rust
# extern crate actix_web;
# extern crate futures;
# use futures::future::result;
use actix_web::*;
use futures::{Future, Stream};


fn index(mut req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    req.from_err()
       .fold((), |_, chunk| {
            println!("Chunk: {:?}", chunk);
            result::<_, error::PayloadError>(Ok(()))
        })
       .map(|_| HttpResponse::Ok().finish())
       .responder()
}
# fn main() {}
```
