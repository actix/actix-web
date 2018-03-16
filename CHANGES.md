# Changes

## 0.4.9 (2018-03-16)

* Allow to disable http/2 support

* Wake payload reading task when data is available

* Fix server keep-alive handling

* Send Query Parameters in client requests #120

* Move brotli encoding to a feature


## 0.4.8 (2018-03-12)

* Allow to set read buffer capacity for server request

* Handle WouldBlock error for socket accept call


## 0.4.7 (2018-03-11)

* Fix panic on unknown content encoding

* Fix connection get closed too early

* Fix streaming response handling for http/2

* Better sleep on error support


## 0.4.6 (2018-03-10)

* Fix client cookie handling

* Fix json content type detection

* Fix CORS middleware #117

* Optimize websockets stream support


## 0.4.5 (2018-03-07)

* Fix compression #103 and #104

* Fix client cookie handling #111

* Non-blocking processing of a `NamedFile`

* Enable compression support for `NamedFile`

* Better support for `NamedFile` type

* Add `ResponseError` impl for `SendRequestError`. This improves ergonomics of the client.
  
* Add native-tls support for client

* Allow client connection timeout to be set #108

* Allow to use std::net::TcpListener for HttpServer

* Handle panics in worker threads


## 0.4.4 (2018-03-04)

* Allow to use Arc<Vec<u8>> as response/request body

* Fix handling of requests with an encoded body with a length > 8192 #93

## 0.4.3 (2018-03-03)

* Fix request body read bug

* Fix segmentation fault #79

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
