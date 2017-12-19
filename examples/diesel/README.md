Diesel's `Getting Started` guide using SQLite for Actix web

## Usage

install `diesel_cli`

```
cargo install diesel_cli --no-default-features --features sqlite
```


```
$ echo "DATABASE_URL=file:test.db" > .env
$ diesel migration run
```
