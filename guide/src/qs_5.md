# Resources and Routes

All resources and routes register for specific application.
Application routes incoming requests based on route criteria which is defined during 
resource registration or path prefix for simple handlers.
Internally *router* is a list of *resources*. Resource is an entry in *route table*
which corresponds to requested URL. 

Prefix handler:

```rust,ignore
fn index(req: Httprequest) -> HttpResponse {
   ...
}

fn main() {
    Application::default("/")
        .handler("/prefix", |req| index)
        .finish();
}
```

In this example `index` get called for any url which starts with `/prefix`. 

Application prefix combines with handler prefix i.e

```rust,ignore
fn main() {
    Application::default("/app")
        .handler("/prefix", |req| index)
        .finish();
}
```

In this example `index` get called for any url which starts with`/app/prefix`. 

Resource contains set of route for same endpoint. Route corresponds to handling 
*HTTP method* by calling *web handler*. Resource select route based on *http method*,
if no route could be matched default response `HTTPMethodNotAllowed` get resturned.

```rust,ignore
fn main() {
    Application::default("/")
        .resource("/prefix", |r| {
           r.get(HTTPOk)
           r.post(HTTPForbidden)
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

A *variable part*is specified in the form {identifier}, where the identifier can be
used later in a request handler to access the matched value for that part. This is
done by looking up the identifier in the `HttpRequest.match_info` object:


```rust
extern crate actix;
use actix_web::*;

fn index(req: Httprequest) -> String {
    format!("Hello, {}", req.match_info.get('name').unwrap())
}

fn main() {
    Application::default("/")
        .resource("/{name}", |r| r.get(index))
        .finish();
}
```

By default, each part matches the regular expression `[^{}/]+`.

You can also specify a custom regex in the form `{identifier:regex}`:

```rust,ignore
fn main() {
    Application::default("/")
        .resource(r"{name:\d+}", |r| r.get(index))
        .finish();
}
```

To match path tail, `{tail:*}` pattern could be used. Tail pattern has to be last
segment in path otherwise it panics.

```rust,ignore
fn main() {
    Application::default("/")
        .resource(r"/test/{tail:*}", |r| r.get(index))
        .finish();
}
```

Above example would match all incoming requests with path such as
'/test/b/c', '/test/index.html', and '/test/etc/test'.
