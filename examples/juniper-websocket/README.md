# juniper-websockect

Simple echo websocket server which handles graphql requests.

## Usage

### server

```bash
cd actix-web/examples/juniper-websocket
cargo run --bin server
# Started http server: 127.0.0.1:8080
```

### web client

- [http://localhost:8080/graphiql](http://localhost:8080/graphiql)

### rust client

```bash
cd actix-web/examples/juniper-websocket
cargo run --bin client
# Started http server: 127.0.0.1:8080
```