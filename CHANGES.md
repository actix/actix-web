# Changes

## 0.4.3 (2018-03-xx)

* Fix request body read bug

* Set reuse address before bind #90


## 0.4.2 (2018-03-02)

* Better naming for websockets implementation

* Add `Pattern::with_prefix()`, make it more usable outside of actix

* Add csrf middleware for filter for cross-site request forgery #89

* Fix disconnect on idle connections


## 0.4.1 (2018-03-01)

* Rename `Route::p()` to `Route::filter()`

* Better naming for http codes

* Fix payload parse in situation when socket data is not ready.

* Fix Session mutable borrow lifetime #87


## 0.4.0 (2018-02-28)

* Actix 0.5 compatibility

* Fix request json/urlencoded loaders

* Simplify HttpServer type definition

* Added HttpRequest::encoding() method

* Added HttpRequest::mime_type() method

* Added HttpRequest::uri_mut(), allows to modify request uri

* Added StaticFiles::index_file()

* Added http client

* Added websocket client

* Added TestServer::ws(), test websockets client

* Added TestServer http client support

* Allow to override content encoding on application level


## 0.3.3 (2018-01-25)

* Stop processing any events after context stop

* Re-enable write back-pressure for h1 connections

* Refactor HttpServer::start_ssl() method

* Upgrade openssl to 0.10


## 0.3.2 (2018-01-21)

* Fix HEAD requests handling

* Log request processing errors

* Always enable content encoding if encoding explicitly selected

* Allow multiple Applications on a single server with different state #49

* CORS middleware: allowed_headers is defaulting to None #50


## 0.3.1 (2018-01-13)

* Fix directory entry path #47

* Do not enable chunked encoding for HTTP/1.0

* Allow explicitly disable chunked encoding


## 0.3.0 (2018-01-12)

* HTTP/2 Support

* Refactor streaming responses

* Refactor error handling

* Asynchronous middlewares

* Refactor logger middleware

* Content compression/decompression (br, gzip, deflate)

* Server multi-threading

* Gracefull shutdown support


## 0.2.1 (2017-11-03)

* Allow to start tls server with `HttpServer::serve_tls`

* Export `Frame` enum

* Add conversion impl from `HttpResponse` and `BinaryBody` to a `Frame`


## 0.2.0 (2017-10-30)

* Do not use `http::Uri` as it can not parse some valid paths

* Refactor response `Body`

* Refactor `RouteRecognizer` usability

* Refactor `HttpContext::write`

* Refactor `Payload` stream

* Re-use `BinaryBody` for `Frame::Payload`

* Stop http actor on `write_eof`

* Fix disconnection handling.


## 0.1.0 (2017-10-23)

* First release
