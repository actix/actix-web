# Resources and Routes

All resources and routes register for specific application.
Application routes incoming requests based on route criteria which is defined during 
resource registration or path prefix for simple handlers.
Internally *router* is a list of *resources*. Resource is an entry in *route table*
which corresponds to requested URL. 

Prefix handler:

```rust
# extern crate actix_web;
# use actix_web::*;
# 
fn index(req: HttpRequest) -> HttpResponse {
   unimplemented!()
}

fn main() {
    Application::default("/")
        .route("/prefix", |r| r.f(index))
        .finish();
}
```

In this example `index` get called for any url which starts with `/prefix`. 

Application prefix combines with handler prefix i.e

```rust
# extern crate actix_web;
# use actix_web::*;
# 
fn index(req: HttpRequest) -> HttpResponse {
   unimplemented!()
}

fn main() {
    Application::default("/app")
        .route("/prefix", |r| r.f(index))
        .finish();
}
```

In this example `index` get called for any url which starts with`/app/prefix`. 

Resource contains set of route for same endpoint. Route corresponds to handling 
*HTTP method* by calling *web handler*. Resource select route based on *http method*,
if no route could be matched default response `HTTPMethodNotAllowed` get resturned.

```rust
# extern crate actix_web;
# use actix_web::*;
# 
fn main() {
    Application::default("/")
        .resource("/prefix", |r| {
           r.method(Method::GET).h(httpcodes::HTTPOk);
           r.method(Method::POST).h(httpcodes::HTTPForbidden);
        })
        .finish();
}
```

[`ApplicationBuilder::resource()` method](../actix_web/dev/struct.ApplicationBuilder.html#method.resource)
accepts configuration function, resource could be configured at once.
Check [`Resource`](../actix-web/target/doc/actix_web/struct.Resource.html) documentation 
for more information.

## Variable resources

Resource may have *variable path*also. For instance, a resource with the 
path '/a/{name}/c' would match all incoming requests with paths such
as '/a/b/c', '/a/1/c', and '/a/etc/c'.

A *variable part* is specified in the form {identifier}, where the identifier can be
used later in a request handler to access the matched value for that part. This is
done by looking up the identifier in the `HttpRequest.match_info` object:

```rust
# extern crate actix_web;
use actix_web::*;

fn index(req: HttpRequest) -> String {
    format!("Hello, {}", &req.match_info()["name"])
}

fn main() {
    Application::default("/")
        .resource("/{name}", |r| r.method(Method::GET).f(index))
        .finish();
}
```

By default, each part matches the regular expression `[^{}/]+`.

You can also specify a custom regex in the form `{identifier:regex}`:

```rust
# extern crate actix_web;
# use actix_web::*;
# fn index(req: HttpRequest) -> String {
#     format!("Hello, {}", &req.match_info()["name"])
# }
# 
fn main() {
    Application::default("/")
        .resource(r"{name:\d+}", |r| r.method(Method::GET).f(index))
        .finish();
}
```

Any matched parameter can be deserialized into specific type if this type 
implements `FromParam` trait. For example most of standard integer types
implements `FromParam` trait. i.e.:

```rust
# extern crate actix_web;
use actix_web::*;

fn index(req: HttpRequest) -> Result<String> {
    let v1: u8 = req.match_info().query("v1")?;
    let v2: u8 = req.match_info().query("v2")?;
    Ok(format!("Values {} {}", v1, v2))
}

fn main() {
    Application::default("/")
        .resource(r"/a/{v1}/{v2}/", |r| r.route().f(index))
        .finish();
}
```

For this example for path '/a/1/2/', values v1 and v2 will resolve to "1" and "2".

It is possible to match path tail with custom `.*` regex.

```rust
# extern crate actix_web;
# use actix_web::*;
# 
# fn index(req: HttpRequest) -> HttpResponse {
#    unimplemented!()
# }
fn main() {
    Application::default("/")
        .resource(r"/test/{tail:.*}", |r| r.method(Method::GET).f(index))
        .finish();
}
```

Above example would match all incoming requests with path such as
'/test/b/c', '/test/index.html', and '/test/etc/test'.

It is possible to create a `PathBuf` from a tail path parameter. The returned `PathBuf` is
percent-decoded. If a segment is equal to "..", the previous segment (if
any) is skipped.

For security purposes, if a segment meets any of the following conditions,
an `Err` is returned indicating the condition met:

  * Decoded segment starts with any of: `.` (except `..`), `*`
  * Decoded segment ends with any of: `:`, `>`, `<`
  * Decoded segment contains any of: `/`
  * On Windows, decoded segment contains any of: '\'
  * Percent-encoding results in invalid UTF8.

As a result of these conditions, a `PathBuf` parsed from request path parameter is
safe to interpolate within, or use as a suffix of, a path without additional checks.

```rust
# extern crate actix_web;
use actix_web::*;
use std::path::PathBuf;

fn index(req: HttpRequest) -> Result<String> {
    let path: PathBuf = req.match_info().query("tail")?;
    Ok(format!("Path {:?}", path))
}

fn main() {
    Application::default("/")
        .resource(r"/a/{tail:.*}", |r| r.method(Method::GET).f(index))
        .finish();
}
```
