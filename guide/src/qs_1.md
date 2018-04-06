# Quick start

## Install Rust

Before we begin, we need to install Rust using the [rustup](https://www.rustup.rs/):

```bash
curl https://sh.rustup.rs -sSf | sh
```

If you already have rustup installed, run this command to ensure you have the latest version of Rust:

```bash
rustup update
```

Actix web framework requires rust version 1.21 and up.

## Running Examples

The fastest way to start experimenting with actix web is to clone the
[repository](https://github.com/actix/actix-web) and run the included examples.

The following set of commands runs the `basics` example:

```bash
git clone https://github.com/actix/actix-web
cd actix-web/examples/basics
cargo run
```

Check [examples/](https://github.com/actix/actix-web/tree/master/examples) directory for more examples.
