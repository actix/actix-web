## 1.0

* Resource registration. 1.0 version uses generalized resource
  registration via `.service()` method.

  instead of

  ```rust
    App.new().resource("/welcome", |r| r.f(welcome))
  ```

  use App's or Scope's `.service()` method. `.service()` method accepts
  object that implements `HttpServiceFactory` trait. By default
  actix-web provides `Resource` and `Scope` services.

  ```rust
    App.new().service(
        web::resource("/welcome")
            .route(web::get().to(welcome))
            .route(web::post().to(post_handler))
  ```

* Scope registration.

  instead of

  ```rust
      let app = App::new().scope("/{project_id}", |scope| {
            scope
                .resource("/path1", |r| r.f(|_| HttpResponse::Ok()))
                .resource("/path2", |r| r.f(|_| HttpResponse::Ok()))
                .resource("/path3", |r| r.f(|_| HttpResponse::MethodNotAllowed()))
      });
  ```

  use `.service()` for registration and `web::scope()` as scope object factory.

  ```rust
      let app = App::new().service(
          web::scope("/{project_id}")
              .service(web::resource("/path1").to(|| HttpResponse::Ok()))
              .service(web::resource("/path2").to(|| HttpResponse::Ok()))
              .service(web::resource("/path3").to(|| HttpResponse::MethodNotAllowed()))
      );
  ```

* `.with()`, `.with_async()` registration methods have been renamed to `.to()` and `.to_async()`.

  instead of

  ```rust
    App.new().resource("/welcome", |r| r.with(welcome))
  ```

  use `.to()` or `.to_async()` methods

  ```rust
    App.new().service(web::resource("/welcome").to(welcome))
  ```

* Passing arguments to handler with extractors, multiple arguments are allowed

  instead of

  ```rust
  fn welcome((body, req): (Bytes, HttpRequest)) -> ... {
    ...
  }
  ```

  use multiple arguments

  ```rust
  fn welcome(body: Bytes, req: HttpRequest) -> ... {
    ...
  }
  ```

* `.f()`, `.a()` and `.h()` handler registration methods have been removed.
  Use `.to()` for handlers and `.to_async()` for async handlers. Handler function
  must use extractors.

  instead of

  ```rust
    App.new().resource("/welcome", |r| r.f(welcome))
  ```

  use App's `to()` or `to_async()` methods

  ```rust
    App.new().service(web::resource("/welcome").to(welcome))
  ```

* `HttpRequest` does not provide access to request's payload stream.

  instead of

  ```rust
  fn index(req: &HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    req
       .payload()
       .from_err()
       .fold((), |_, chunk| {
            ...
        })
       .map(|_| HttpResponse::Ok().finish())
       .responder()
  }
  ```

  use `Payload` extractor

  ```rust
  fn index(stream: web::Payload) -> impl Future<Item=HttpResponse, Error=Error> {
     stream
       .from_err()
       .fold((), |_, chunk| {
            ...
        })
       .map(|_| HttpResponse::Ok().finish())
  }
  ```

* `State` is now `Data`.  You register Data during the App initialization process
  and then access it from handlers either using a Data extractor or using
  HttpRequest's api.

  instead of

  ```rust
    App.with_state(T)
  ```

  use App's `data` method

  ```rust
	App.new()
       .data(T)
  ```

  and either use the Data extractor within your handler

  ```rust
    use actix_web::web::Data;

	fn endpoint_handler(Data<T>)){
      ...
    }
  ```

  .. or access your Data element from the HttpRequest

  ```rust
	fn endpoint_handler(req: HttpRequest) {
		let data: Option<Data<T>> = req.app_data::<T>();
    }
  ```


* AsyncResponder is removed, use `.to_async()` registration method and `impl Future<>` as result type.

  instead of

  ```rust
	use actix_web::AsyncResponder;

    fn endpoint_handler(...) -> impl Future<Item=HttpResponse, Error=Error>{
		...
        .responder()
	}
  ```

  .. simply omit AsyncResponder and the corresponding responder() finish method


* Middleware

  instead of

  ```rust
      let app = App::new()
           .middleware(middleware::Logger::default())
  ```

  use `.wrap()` method

  ```rust
      let app = App::new()
           .wrap(middleware::Logger::default())
           .route("/index.html", web::get().to(index));
  ```

* `HttpRequest::body()`, `HttpRequest::urlencoded()`, `HttpRequest::json()`, `HttpRequest::multipart()`
  method have been removed. Use `Bytes`, `String`, `Form`, `Json`, `Multipart` extractors instead.

  instead of

  ```rust
  fn index(req: &HttpRequest) -> Responder {
     req.body()
       .and_then(|body| {
          ...
       })
  }
  ```

  use

  ```rust
  fn index(body: Bytes) -> Responder {
     ...
  }
  ```

* `actix_web::server` module has been removed. To start http server use `actix_web::HttpServer` type

* StaticFiles and NamedFile has been move to separate create.

  instead of `use actix_web::fs::StaticFile`

  use `use actix_files::Files`

  instead of `use actix_web::fs::Namedfile`

  use `use actix_files::NamedFile`

* Multipart has been move to separate create.

  instead of `use actix_web::multipart::Multipart`

  use `use actix_multipart::Multipart`

* Response compression is not enabled by default.
  To enable, use `Compress` middleware, `App::new().wrap(Compress::default())`.

* Session middleware moved to actix-session crate

* Actors support have been moved to `actix-web-actors` crate

* Custom Error

  Instead of error_response method alone, ResponseError now provides two methods: error_response and render_response respectively. Where, error_response creates the error response and render_response returns the error response to the caller. 

  Simplest migration from 0.7 to 1.0 shall include below method to the custom implementation of ResponseError:

  ```rust
  fn render_response(&self) -> HttpResponse {
    self.error_response()
  }
  ```

## 0.7.15

* The `' '` character is not percent decoded anymore before matching routes. If you need to use it in
  your routes, you should use `%20`.

  instead of

    ```rust
    fn main() {
         let app = App::new().resource("/my index", |r| {
             r.method(http::Method::GET)
                    .with(index);
         });
    }
    ```

  use

    ```rust
    fn main() {
         let app = App::new().resource("/my%20index", |r| {
             r.method(http::Method::GET)
                    .with(index);
         });
    }
    ```

* If you used `AsyncResult::async` you need to replace it with `AsyncResult::future`


## 0.7.4

* `Route::with_config()`/`Route::with_async_config()` always passes configuration objects as tuple
  even for handler with one parameter.


## 0.7

* `HttpRequest` does not implement `Stream` anymore. If you need to read request payload
  use `HttpMessage::payload()` method.
  
  instead of
  
    ```rust
    fn index(req: HttpRequest) -> impl Responder {
         req
            .from_err()
            .fold(...)
            ....
    }
    ```

  use `.payload()`

    ```rust
    fn index(req: HttpRequest) -> impl Responder {
         req
            .payload()  // <- get request payload stream
            .from_err()
            .fold(...)
            ....
    }
    ```

* [Middleware](https://actix.rs/actix-web/actix_web/middleware/trait.Middleware.html)
  trait uses `&HttpRequest` instead of `&mut HttpRequest`.

* Removed `Route::with2()` and `Route::with3()` use tuple of extractors instead.
    
    instead of 

    ```rust
    fn index(query: Query<..>, info: Json<MyStruct) -> impl Responder {}
    ```

    use tuple of extractors and use `.with()` for registration:

    ```rust
    fn index((query, json): (Query<..>, Json<MyStruct)) -> impl Responder {}
    ```

* `Handler::handle()` uses `&self` instead of `&mut self`

* `Handler::handle()` accepts reference to `HttpRequest<_>` instead of value

* Removed deprecated `HttpServer::threads()`, use 
  [HttpServer::workers()](https://actix.rs/actix-web/actix_web/server/struct.HttpServer.html#method.workers) instead.

* Renamed `client::ClientConnectorError::Connector` to
  `client::ClientConnectorError::Resolver`

* `Route::with()` does not return `ExtractorConfig`, to configure
  extractor use `Route::with_config()`

    instead of 

    ```rust
    fn main() {
         let app = App::new().resource("/index.html", |r| {
             r.method(http::Method::GET)
                    .with(index)
                    .limit(4096);  // <- limit size of the payload
         });
    }
    ```
    
    use 
    
    ```rust
  
    fn main() {
         let app = App::new().resource("/index.html", |r| {
             r.method(http::Method::GET)
                    .with_config(index, |cfg| { // <- register handler
                       cfg.limit(4096);  // <- limit size of the payload
                     })
         });
    }
    ```

* `Route::with_async()` does not return `ExtractorConfig`, to configure
  extractor use `Route::with_async_config()`


## 0.6

* `Path<T>` extractor return `ErrorNotFound` on failure instead of `ErrorBadRequest`

* `ws::Message::Close` now includes optional close reason.
  `ws::CloseCode::Status` and `ws::CloseCode::Empty` have been removed.

* `HttpServer::threads()` renamed to `HttpServer::workers()`.

* `HttpServer::start_ssl()` and `HttpServer::start_tls()` deprecated.
  Use `HttpServer::bind_ssl()` and `HttpServer::bind_tls()` instead.

* `HttpRequest::extensions()` returns read only reference to the request's Extension
  `HttpRequest::extensions_mut()` returns mutable reference.

* Instead of 

   `use actix_web::middleware::{
        CookieSessionBackend, CookieSessionError, RequestSession,
        Session, SessionBackend, SessionImpl, SessionStorage};`
                                
  use `actix_web::middleware::session`

   `use actix_web::middleware::session{CookieSessionBackend, CookieSessionError,
        RequestSession, Session, SessionBackend, SessionImpl, SessionStorage};`

* `FromRequest::from_request()` accepts mutable reference to a request

* `FromRequest::Result` has to implement `Into<Reply<Self>>`

* [`Responder::respond_to()`](
  https://actix.rs/actix-web/actix_web/trait.Responder.html#tymethod.respond_to)
  is generic over `S`

*  Use `Query` extractor instead of HttpRequest::query()`.

   ```rust
   fn index(q: Query<HashMap<String, String>>) -> Result<..> {
       ...
   }
   ```

   or

   ```rust
   let q = Query::<HashMap<String, String>>::extract(req);
   ```

* Websocket operations are implemented as `WsWriter` trait.
  you need to use `use actix_web::ws::WsWriter`


## 0.5

* `HttpResponseBuilder::body()`, `.finish()`, `.json()`
   methods return `HttpResponse` instead of `Result<HttpResponse>`

* `actix_web::Method`, `actix_web::StatusCode`, `actix_web::Version`
   moved to `actix_web::http` module

* `actix_web::header` moved to `actix_web::http::header`

* `NormalizePath` moved to `actix_web::http` module

* `HttpServer` moved to `actix_web::server`, added new `actix_web::server::new()` function,
  shortcut for `actix_web::server::HttpServer::new()`

* `DefaultHeaders` middleware does not use separate builder, all builder methods moved to type itself

* `StaticFiles::new()`'s show_index parameter removed, use `show_files_listing()` method instead.

* `CookieSessionBackendBuilder` removed, all methods moved to `CookieSessionBackend` type

* `actix_web::httpcodes` module is deprecated, `HttpResponse::Ok()`, `HttpResponse::Found()` and other `HttpResponse::XXX()`
   functions should be used instead

* `ClientRequestBuilder::body()` returns `Result<_, actix_web::Error>`
  instead of `Result<_, http::Error>`

* `Application` renamed to a `App`

* `actix_web::Reply`, `actix_web::Resource` moved to `actix_web::dev`
