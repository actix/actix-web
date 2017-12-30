# tls example

## Usage

### server

```bash
cd actix-web/examples/tls
cargo run (or ``cargo watch -x run``)
# Started http server: 127.0.0.1:8443
```

### web client

- curl: ``curl -v https://127.0.0.1:8443/index.html --compress -k``
- browser: [https://127.0.0.1:8443/index.html](https://127.0.0.1:8080/index.html)
