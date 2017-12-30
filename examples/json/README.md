# json

Json's `Getting Started` guide using json (serde-json or json-rust) for Actix web

## Usage

### server

```bash
cd actix-web/examples/json
cargo run
# Started http server: 127.0.0.1:8080
```

### client

With [Postman](https://www.getpostman.com/) or [Rested](moz-extension://60daeb1c-5b1b-4afd-9842-0579ed34dfcb/dist/index.html)

- POST / (embed serde-json):

  - method : ``POST``
  - url : ``http://127.0.0.1:8080/``
  - header : ``Content-Type`` = ``application/json``
  - body (raw) : ``{"name": "Test user", "number": 100}``

- POST /manual (manual serde-json):

  - method : ``POST``
  - url : ``http://127.0.0.1:8080/manual``
  - header : ``Content-Type`` = ``application/json``
  - body (raw) : ``{"name": "Test user", "number": 100}``

- POST /mjsonrust (manual json-rust):

  - method : ``POST``
  - url : ``http://127.0.0.1:8080/mjsonrust``
  - header : ``Content-Type`` = ``application/json``
  - body (raw) : ``{"name": "Test user", "number": 100}`` (you can also test ``{notjson}``)
