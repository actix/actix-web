# diesel

Diesel's `Getting Started` guide using SQLite for Actix web

## Usage

install `diesel_cli`

```bash
cargo install diesel_cli --no-default-features --features sqlite
```

```bash
echo "DATABASE_URL=file:test.db" > .env
diesel migration run
```

## Postgresql

You will also find another complete example of diesel+postgresql on      [https://github.com/TechEmpower/FrameworkBenchmarks/tree/master/frameworks/Rust/actix](https://github.com/TechEmpower/FrameworkBenchmarks/tree/master/frameworks/Rust/actix)