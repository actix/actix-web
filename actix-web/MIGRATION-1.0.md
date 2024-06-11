## 1.0.1

- Cors middleware has been moved to `actix-cors` crate

  instead of

  ```rust
  use actix_web::middleware::cors::Cors;
  ```

  use

  ```rust
  use actix_cors::Cors;
  ```

- Identity middleware has been moved to `actix-identity` crate

  instead of

  ```rust
  use actix_web::middleware::identity::{Identity, CookieIdentityPolicy, IdentityService};
  ```

  use

  ```rust
  use actix_identity::{Identity, CookieIdentityPolicy, IdentityService};
  ```

## 1.0.0

- Extractor configuration. In version 1.0 this is handled with the new `Data` mechanism for both setting and retrieving the configuration

  instead of

  ```rust

  #[derive(Default)]
  struct ExtractorConfig {
     config: String,
  }

  impl FromRequest for YourExtractor {
     type Config = ExtractorConfig;
     type Result = Result<YourExtractor, Error>;

     fn from_request(req: &HttpRequest, cfg: &Self::Config) -> Self::Result {
         println!("use the config: {:?}", cfg.config);
         ...
     }
  }

  App::new().resource("/route_with_config", |r| {
     r.post().with_config(handler_fn, |cfg| {
         cfg.0.config = "test".to_string();
     })
  })

  ```

  use the HttpRequest to get the configuration like any other `Data` with `req.app_data::<C>()` and set it with the `data()` method on the `resource`

  ```rust
  #[derive(Default)]
  struct ExtractorConfig {
     config: String,
  }

  impl FromRequest for YourExtractor {
     type Error = Error;
     type Future = Result<Self, Self::Error>;
     type Config = ExtractorConfig;

     fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
         let cfg = req.app_data::<ExtractorConfig>();
         println!("config data?: {:?}", cfg.unwrap().role);
         ...
     }
  }

  App::new().service(
     resource("/route_with_config")
         .data(ExtractorConfig {
             config: "test".to_string(),
         })
         .route(post().to(handler_fn)),
  )
  ```

- Resource registration. 1.0 version uses generalized resource registration via `.service()` method.

  instead of

  ```rust
    App.new().resource("/welcome", |r| r.f(welcome))
  ```

  use App's or Scope's `.service()` method. `.service()` method accepts object that implements `HttpServiceFactory` trait. By default actix-web provides `Resource` and `Scope` services.

  ```rust
    App.new().service(
        web::resource("/welcome")
            .route(web::get().to(welcome))
            .route(web::post().to(post_handler))
  ```

- Scope registration.

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

- `.with()`, `.with_async()` registration methods have been renamed to `.to()` and `.to_async()`.

  instead of

  ```rust
    App.new().resource("/welcome", |r| r.with(welcome))
  ```

  use `.to()` or `.to_async()` methods

  ```rust
    App.new().service(web::resource("/welcome").to(welcome))
  ```

- Passing arguments to handler with extractors, multiple arguments are allowed

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

- `.f()`, `.a()` and `.h()` handler registration methods have been removed. Use `.to()` for handlers and `.to_async()` for async handlers. Handler function must use extractors.

  instead of

  ```rust
    App.new().resource("/welcome", |r| r.f(welcome))
  ```

  use App's `to()` or `to_async()` methods

  ```rust
    App.new().service(web::resource("/welcome").to(welcome))
  ```

- `HttpRequest` does not provide access to request's payload stream.

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

- `State` is now `Data`. You register Data during the App initialization process and then access it from handlers either using a Data extractor or using HttpRequest's api.

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

- AsyncResponder is removed, use `.to_async()` registration method and `impl Future<>` as result type.

  instead of

  ```rust
  use actix_web::AsyncResponder;

    fn endpoint_handler(...) -> impl Future<Item=HttpResponse, Error=Error>{
  	...
        .responder()
  }
  ```

  .. simply omit AsyncResponder and the corresponding responder() finish method

- Middleware

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

- `HttpRequest::body()`, `HttpRequest::urlencoded()`, `HttpRequest::json()`, `HttpRequest::multipart()` method have been removed. Use `Bytes`, `String`, `Form`, `Json`, `Multipart` extractors instead.

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

- `actix_web::server` module has been removed. To start http server use `actix_web::HttpServer` type

- StaticFiles and NamedFile have been moved to a separate crate.

  instead of `use actix_web::fs::StaticFile`

  use `use actix_files::Files`

  instead of `use actix_web::fs::Namedfile`

  use `use actix_files::NamedFile`

- Multipart has been moved to a separate crate.

  instead of `use actix_web::multipart::Multipart`

  use `use actix_multipart::Multipart`

- Response compression is not enabled by default. To enable, use `Compress` middleware, `App::new().wrap(Compress::default())`.

- Session middleware moved to actix-session crate

- Actors support have been moved to `actix-web-actors` crate

- Custom Error

  Instead of error_response method alone, ResponseError now provides two methods: error_response and render_response respectively. Where, error_response creates the error response and render_response returns the error response to the caller.

  Simplest migration from 0.7 to 1.0 shall include below method to the custom implementation of ResponseError:

  ```rust
  fn render_response(&self) -> HttpResponse {
    self.error_response()
  }
  ```
