# Changes

## 0.3.2 (2018-01-xx)

* Can't have multiple Applications on a single server with different state #49


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
