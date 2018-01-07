# diesel

Diesel's `Getting Started` guide using SQLite for Actix web

## Usage

### init database sqlite

```bash
cargo install diesel_cli --no-default-features --features sqlite
cd actix-web/examples/diesel
echo "DATABASE_URL=file:test.db" > .env
diesel migration run
```

### server

```bash
# if ubuntu : sudo apt-get install libsqlite3-dev
# if fedora : sudo dnf install libsqlite3x-devel
cd actix-web/examples/diesel
cargo run (or ``cargo watch -x run``)
# Started http server: 127.0.0.1:8080
```

### web client

[http://127.0.0.1:8080/NAME](http://127.0.0.1:8080/NAME)

### sqlite client

```bash
# if ubuntu : sudo apt-get install sqlite3
# if fedora : sudo dnf install sqlite3x
sqlite3 test.db
sqlite> .tables
sqlite> select * from users;
```


## Postgresql

You will also find another complete example of diesel+postgresql on      [https://github.com/TechEmpower/FrameworkBenchmarks/tree/master/frameworks/Rust/actix](https://github.com/TechEmpower/FrameworkBenchmarks/tree/master/frameworks/Rust/actix)