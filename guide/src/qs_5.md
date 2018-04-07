# URL Dispatch

URL dispatch provides a simple way for mapping URLs to `Handler` code using a simple pattern
matching language. If one of the patterns matches the path information associated with a request,
a particular handler object is invoked.

> A handler is a specific object that implements the
> `Handler` trait, defined in your application, that receives the request and returns
> a response object. More information is available in the [handler section](../qs_4.html).

## Resource configuration

Resource configuration is the act of adding a new resources to an application.
A resource has a name, which acts as an identifier to be used for URL generation.
The name also allows developers to add routes to existing resources.
A resource also has a pattern, meant to match against the *PATH* portion of a *URL*.
It does not match against the *QUERY* portion (the portion following the scheme and
port, e.g., */foo/bar* in the *URL* *http://localhost:8080/foo/bar?q=value*).

The [App::route](../actix_web/struct.App.html#method.route) method provides
simple way of registering routes. This method adds a single route to application
routing table. This method accepts a *path pattern*,
*http method* and a handler function. `route()` method could be called multiple times
for the same path, in that case, multiple routes register for the same resource path.

```rust
# extern crate actix_web;
use actix_web::{App, HttpRequest, HttpResponse, http::Method};

fn index(req: HttpRequest) -> HttpResponse {
   unimplemented!()
}

fn main() {
    App::new()
        .route("/user/{name}", Method::GET, index)
        .route("/user/{name}", Method::POST, index)
        .finish();
}
```

While *App::route()* provides simple way of registering routes, to access
complete resource configuration, different method has to be used.
The [App::resource](../actix_web/struct.App.html#method.resource) method
adds a single resource to application routing table. This method accepts a *path pattern*
and a resource configuration function.

```rust
# extern crate actix_web;
use actix_web::{App, HttpRequest, HttpResponse, http::Method};

fn index(req: HttpRequest) -> HttpResponse {
   unimplemented!()
}

fn main() {
    App::new()
        .resource("/prefix", |r| r.f(index))
        .resource("/user/{name}",
             |r| r.method(Method::GET).f(|req| HttpResponse::Ok()))
        .finish();
}
```

The *Configuration function* has the following type:

```rust,ignore
   FnOnce(&mut Resource<_>) -> ()
```

The *Configuration function* can set a name and register specific routes.
If a resource does not contain any route or does not have any matching routes, it
returns *NOT FOUND* http response.

## Configuring a Route

Resource contains a set of routes. Each route in turn has a set of predicates and a handler.
New routes can be created with `Resource::route()` method which returns a reference
to new *Route* instance. By default the *route* does not contain any predicates, so matches
all requests and the default handler is `HttpNotFound`.

The application routes incoming requests based on route criteria which are defined during
resource registration and route registration. Resource matches all routes it contains in
the order the routes were registered via `Resource::route()`.

> A *Route* can contain any number of *predicates* but only one handler.

```rust
# extern crate actix_web;
# use actix_web::*;

fn main() {
    App::new()
        .resource("/path", |resource|
            resource.route()
              .filter(pred::Get())
              .filter(pred::Header("content-type", "text/plain"))
              .f(|req| HttpResponse::Ok())
        )
        .finish();
}
```

In this example, `HttpResponse::Ok()` is returned for *GET* requests.
If a request contains `Content-Type` header, the value of this header is *text/plain*,
and path equals to `/path`, Resource calls handle of the first matching route.

If a resource can not match any route, a "NOT FOUND" response is returned.

[*Resource::route()*](../actix_web/struct.Resource.html#method.route) returns a
[*Route*](../actix_web/struct.Route.html) object. Route can be configured with a
builder-like pattern. Following configuration methods are available:

* [*Route::filter()*](../actix_web/struct.Route.html#method.filter) registers a new predicate.
  Any number of predicates can be registered for each route.

* [*Route::f()*](../actix_web/struct.Route.html#method.f) registers handler function
  for this route. Only one handler can be registered. Usually handler registration
  is the last config operation. Handler function can be a function or closure and has the type
  `Fn(HttpRequest<S>) -> R + 'static`

* [*Route::h()*](../actix_web/struct.Route.html#method.h) registers a handler object
  that implements the `Handler` trait. This is similar to `f()` method - only one handler can
  be registered. Handler registration is the last config operation.

* [*Route::a()*](../actix_web/struct.Route.html#method.a) registers an async handler
  function for this route. Only one handler can be registered. Handler registration
  is the last config operation. Handler function can be a function or closure and has the type
  `Fn(HttpRequest<S>) -> Future<Item = HttpResponse, Error = Error> + 'static`

## Route matching

The main purpose of route configuration is to match (or not match) the request's `path`
against a URL path pattern. `path` represents the path portion of the URL that was requested.

The way that *actix* does this is very simple. When a request enters the system,
for each resource configuration declaration present in the system, actix checks
the request's path against the pattern declared. This checking happens in the order that
the routes were declared via `App::resource()` method. If resource can not be found,
the *default resource* is used as the matched resource.

When a route configuration is declared, it may contain route predicate arguments. All route
predicates associated with a route declaration must be `true` for the route configuration to
be used for a given request during a check. If any predicate in the set of route predicate
arguments provided to a route configuration returns `false` during a check, that route is
skipped and route matching continues through the ordered set of routes.

If any route matches, the route matching process stops and the handler associated with
the route is invoked. If no route matches after all route patterns are exhausted, a *NOT FOUND* response get returned.

## Resource pattern syntax

The syntax of the pattern matching language used by actix in the pattern
argument is straightforward.

The pattern used in route configuration may start with a slash character. If the pattern
does not start with a slash character, an implicit slash will be prepended
to it at matching time. For example, the following patterns are equivalent:

```
{foo}/bar/baz
```

and:

```
/{foo}/bar/baz
```

A *variable part* (replacement marker) is specified in the form *{identifier}*,
where this means "accept any characters up to the next slash character and use this
as the name in the `HttpRequest.match_info()` object".

A replacement marker in a pattern matches the regular expression `[^{}/]+`.

A match_info is the `Params` object representing the dynamic parts extracted from a
*URL* based on the routing pattern. It is available as *request.match_info*. For example, the
following pattern defines one literal segment (foo) and two replacement markers (baz, and bar):

```
foo/{baz}/{bar}
```

The above pattern will match these URLs, generating the following match information:

```
foo/1/2        -> Params {'baz':'1', 'bar':'2'}
foo/abc/def    -> Params {'baz':'abc', 'bar':'def'}
```

It will not match the following patterns however:

```
foo/1/2/        -> No match (trailing slash)
bar/abc/def     -> First segment literal mismatch
```

The match for a segment replacement marker in a segment will be done only up to
the first non-alphanumeric character in the segment in the pattern. So, for instance,
if this route pattern was used:

```
foo/{name}.html
```

The literal path */foo/biz.html* will match the above route pattern, and the match result
will be `Params{'name': 'biz'}`. However, the literal path */foo/biz* will not match,
because it does not contain a literal *.html* at the end of the segment represented
by *{name}.html* (it only contains biz, not biz.html).

To capture both segments, two replacement markers can be used:

```
foo/{name}.{ext}
```

The literal path */foo/biz.html* will match the above route pattern, and the match
result will be *Params{'name': 'biz', 'ext': 'html'}*. This occurs because there is a
literal part of *.* (period) between the two replacement markers *{name}* and *{ext}*.

Replacement markers can optionally specify a regular expression which will be used to decide
whether a path segment should match the marker. To specify that a replacement marker should
match only a specific set of characters as defined by a regular expression, you must use a
slightly extended form of replacement marker syntax. Within braces, the replacement marker
name must be followed by a colon, then directly thereafter, the regular expression. The default
regular expression associated with a replacement marker *[^/]+* matches one or more characters
which are not a slash. For example, under the hood, the replacement marker *{foo}* can more
verbosely be spelled as *{foo:[^/]+}*. You can change this to be an arbitrary regular expression
to match an arbitrary sequence of characters, such as *{foo:\d+}* to match only digits.

Segments must contain at least one character in order to match a segment replacement marker.
For example, for the URL */abc/*:

* */abc/{foo}* will not match.
* */{foo}/* will match.

> **Note**: path will be URL-unquoted and decoded into valid unicode string before
> matching pattern and values representing matched path segments will be URL-unquoted too.

So for instance, the following pattern:

```
foo/{bar}
```

When matching the following URL:

```
http://example.com/foo/La%20Pe%C3%B1a
```

The matchdict will look like so (the value is URL-decoded):

```
Params{'bar': 'La Pe\xf1a'}
```

Literal strings in the path segment should represent the decoded value of the
path provided to actix. You don't want to use a URL-encoded value in the pattern.
For example, rather than this:

```
/Foo%20Bar/{baz}
```

You'll want to use something like this:

```
/Foo Bar/{baz}
```

It is possible to get "tail match". For this purpose custom regex has to be used.

```
foo/{bar}/{tail:.*}
```

The above pattern will match these URLs, generating the following match information:

```
foo/1/2/           -> Params{'bar':'1', 'tail': '2/'}
foo/abc/def/a/b/c  -> Params{'bar':u'abc', 'tail': 'def/a/b/c'}
```

## Match information

All values representing matched path segments are available in
[`HttpRequest::match_info`](../actix_web/struct.HttpRequest.html#method.match_info).
Specific values can be retrieved with
[`Params::get()`](../actix_web/dev/struct.Params.html#method.get).

Any matched parameter can be deserialized into a specific type if the type
implements the `FromParam` trait. For example most standard integer types
the trait, i.e.:

```rust
# extern crate actix_web;
use actix_web::*;

fn index(req: HttpRequest) -> Result<String> {
    let v1: u8 = req.match_info().query("v1")?;
    let v2: u8 = req.match_info().query("v2")?;
    Ok(format!("Values {} {}", v1, v2))
}

fn main() {
    App::new()
        .resource(r"/a/{v1}/{v2}/", |r| r.f(index))
        .finish();
}
```

For this example for path '/a/1/2/', values v1 and v2 will resolve to "1" and "2".

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
use std::path::PathBuf;
use actix_web::{App, HttpRequest, Result, http::Method};

fn index(req: HttpRequest) -> Result<String> {
    let path: PathBuf = req.match_info().query("tail")?;
    Ok(format!("Path {:?}", path))
}

fn main() {
    App::new()
        .resource(r"/a/{tail:.*}", |r| r.method(Method::GET).f(index))
        .finish();
}
```

List of `FromParam` implementations can be found in
[api docs](../actix_web/dev/trait.FromParam.html#foreign-impls)

## Path information extractor

Actix provides functionality for type safe request path information extraction.
It uses *serde* package as a deserialization library.
[Path](../actix_web/struct.Path.html) extracts information, the destination type
has to implement *serde's *`Deserialize` trait.

```rust
# extern crate actix_web;
#[macro_use] extern crate serde_derive;
use actix_web::{App, Path, Result, http::Method};

#[derive(Deserialize)]
struct Info {
    username: String,
}

// extract path info using serde
fn index(info: Path<Info>) -> Result<String> {
    Ok(format!("Welcome {}!", info.username))
}

fn main() {
    let app = App::new()
        .resource("/{username}/index.html",    // <- define path parameters
                  |r| r.method(Method::GET).with(index));
}
```

It also possible to extract path information to a tuple. In this case, you don't need
to define an extra type; use a tuple as a `Path` generic type.

Here is previous example re-written using tuple instead of specific type.

```rust
# extern crate actix_web;
use actix_web::{App, Path, Result, http::Method};

// extract path info using serde
fn index(info: Path<(String, u32)>) -> Result<String> {
    Ok(format!("Welcome {}! id: {}", info.0, info.1))
}

fn main() {
    let app = App::new()
        .resource("/{username}/{id}/index.html",    // <- define path parameters
                  |r| r.method(Method::GET).with(index));
}
```

[Query](../actix_web/struct.Query.html) provides similar functionality for
request query parameters.


## Generating resource URLs

Use the [HttpRequest.url_for()](../actix_web/struct.HttpRequest.html#method.url_for)
method to generate URLs based on resource patterns. For example, if you've configured a
resource with the name "foo" and the pattern "{a}/{b}/{c}", you might do this:

```rust
# extern crate actix_web;
# use actix_web::{App, HttpRequest, HttpResponse, http::Method};
#
fn index(req: HttpRequest) -> HttpResponse {
    let url = req.url_for("foo", &["1", "2", "3"]); // <- generate url for "foo" resource
    HttpResponse::Ok().into()
}

fn main() {
    let app = App::new()
        .resource("/test/{a}/{b}/{c}", |r| {
             r.name("foo");  // <- set resource name, then it could be used in `url_for`
             r.method(Method::GET).f(|_| HttpResponse::Ok());
        })
        .finish();
}
```

This would return something like the string *http://example.com/test/1/2/3* (at least if
the current protocol and hostname implied http://example.com).
`url_for()` method returns [*Url object*](https://docs.rs/url/1.6.0/url/struct.Url.html) so you
can modify this url (add query parameters, anchor, etc).
`url_for()` could be called only for *named* resources otherwise error get returned.

## External resources

Resources that are valid URLs, can be registered as external resources. They are useful
for URL generation purposes only and are never considered for matching at request time.

```rust
# extern crate actix_web;
use actix_web::{App, HttpRequest, HttpResponse, Error};

fn index(mut req: HttpRequest) -> Result<HttpResponse, Error> {
    let url = req.url_for("youtube", &["oHg5SJYRHA0"])?;
    assert_eq!(url.as_str(), "https://youtube.com/watch/oHg5SJYRHA0");
    Ok(HttpResponse::Ok().into())
}

fn main() {
    let app = App::new()
        .resource("/index.html", |r| r.f(index))
        .external_resource("youtube", "https://youtube.com/watch/{video_id}")
        .finish();
}
```

## Path normalization and redirecting to slash-appended routes

By normalizing it means:

* Add a trailing slash to the path.
* Double slashes are replaced by one.

The handler returns as soon as it finds a path that resolves
correctly. The order if all enable is 1) merge, 3) both merge and append
and 3) append. If the path resolves with
at least one of those conditions, it will redirect to the new path.

If *append* is *true*, append slash when needed. If a resource is
defined with trailing slash and the request doesn't have one, it will
be appended automatically.

If *merge* is *true*, merge multiple consecutive slashes in the path into one.

This handler designed to be used as a handler for application's *default resource*.

```rust
# extern crate actix_web;
# #[macro_use] extern crate serde_derive;
# use actix_web::*;
use actix_web::http::NormalizePath;
#
# fn index(req: HttpRequest) -> HttpResponse {
#    HttpResponse::Ok().into()
# }
fn main() {
    let app = App::new()
        .resource("/resource/", |r| r.f(index))
        .default_resource(|r| r.h(NormalizePath::default()))
        .finish();
}
```

In this example `/resource`, `//resource///` will be redirected to `/resource/`.

In this example, the path normalization handler is registered for all methods,
but you should not rely on this mechanism to redirect *POST* requests. The redirect of the
slash-appending *Not Found* will turn a *POST* request into a GET, losing any
*POST* data in the original request.

It is possible to register path normalization only for *GET* requests only:

```rust
# extern crate actix_web;
# #[macro_use] extern crate serde_derive;
use actix_web::{App, HttpRequest, http::Method, http::NormalizePath};
#
# fn index(req: HttpRequest) -> &'static str {
#    "test"
# }
fn main() {
    let app = App::new()
        .resource("/resource/", |r| r.f(index))
        .default_resource(|r| r.method(Method::GET).h(NormalizePath::default()))
        .finish();
}
```

## Using an Application Prefix to Compose Applications

The `App::prefix()` method allows to set a specific application prefix.
This prefix represents a resource prefix that will be prepended to all resource patterns added
by the resource configuration. This can be used to help mount a set of routes at a different
location than the included callable's author intended while still maintaining the same
resource names.

For example:

```rust
# extern crate actix_web;
# use actix_web::*;
#
fn show_users(req: HttpRequest) -> HttpResponse {
   unimplemented!()
}

fn main() {
    App::new()
        .prefix("/users")
        .resource("/show", |r| r.f(show_users))
        .finish();
}
```

In the above example, the *show_users* route will have an effective route pattern of
*/users/show* instead of */show* because the application's prefix argument will be prepended
to the pattern. The route will then only match if the URL path is */users/show*,
and when the `HttpRequest.url_for()` function is called with the route name show_users,
it will generate a URL with that same path.

## Custom route predicates

You can think of a predicate as a simple function that accepts a *request* object reference
and returns *true* or *false*. Formally, a predicate is any object that implements the
[`Predicate`](../actix_web/pred/trait.Predicate.html) trait. Actix provides
several predicates, you can check [functions section](../actix_web/pred/index.html#functions)
of api docs.

Here is a simple predicate that check that a request contains a specific *header*:

```rust
# extern crate actix_web;
# use actix_web::*;
use actix_web::{http, pred::Predicate, App, HttpRequest};

struct ContentTypeHeader;

impl<S: 'static> Predicate<S> for ContentTypeHeader {

    fn check(&self, req: &mut HttpRequest<S>) -> bool {
       req.headers().contains_key(http::header::CONTENT_TYPE)
    }
}

fn main() {
    App::new()
        .resource("/index.html", |r|
           r.route()
              .filter(ContentTypeHeader)
              .f(|_| HttpResponse::Ok()));
}
```

In this example, *index* handler will be called only if request contains *CONTENT-TYPE* header.

Predicates have access to the application's state via `HttpRequest::state()`.
Also predicates can store extra information in
[request extensions](../actix_web/struct.HttpRequest.html#method.extensions).

### Modifying predicate values

You can invert the meaning of any predicate value by wrapping it in a `Not` predicate.
For example, if you want to return "METHOD NOT ALLOWED" response for all methods
except "GET":

```rust
# extern crate actix_web;
# extern crate http;
# use actix_web::*;
use actix_web::{pred, App, HttpResponse};

fn main() {
    App::new()
        .resource("/index.html", |r|
           r.route()
              .filter(pred::Not(pred::Get()))
              .f(|req| HttpResponse::MethodNotAllowed()))
        .finish();
}
```

The `Any` predicate accepts a list of predicates and matches if any of the supplied
predicates match. i.e:

```rust,ignore
    pred::Any(pred::Get()).or(pred::Post())
```

The `All` predicate accepts a list of predicates and matches if all of the supplied
predicates match. i.e:

```rust,ignore
    pred::All(pred::Get()).and(pred::Header("content-type", "plain/text"))
```

## Changing the default Not Found response

If the path pattern can not be found in the routing table or a resource can not find matching
route, the default resource is used. The default response is *NOT FOUND*.
It is possible to override the *NOT FOUND* response with `App::default_resource()`.
This method accepts a *configuration function* same as normal resource configuration
with `App::resource()` method.

```rust
# extern crate actix_web;
use actix_web::{App, HttpResponse, http::Method, pred};

fn main() {
    App::new()
        .default_resource(|r| {
              r.method(Method::GET).f(|req| HttpResponse::NotFound());
              r.route().filter(pred::Not(pred::Get()))
                  .f(|req| HttpResponse::MethodNotAllowed());
         })
#        .finish();
}
```
