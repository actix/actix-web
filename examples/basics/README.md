# basics

## Usage

### server

```bash
cd actix-web/examples/basics
cargo run
# Started http server: 127.0.0.1:8080
```

### web client

- [http://localhost:8080/index.html](http://localhost:8080/index.html)
- [http://localhost:8080/async/bob](http://localhost:8080/async/bob)
- [http://localhost:8080/user/bob/](http://localhost:8080/user/bob/) plain/text download
- [http://localhost:8080/test](http://localhost:8080/test) (return status switch GET or POST or other)
- [http://localhost:8080/static/index.html](http://localhost:8080/static/index.html)
- [http://localhost:8080/static/notexit](http://localhost:8080/static/notexit) display 404 page
