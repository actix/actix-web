# Quick start

Before you can start writing a actix web application, you’ll need a version of Rust installed.
We recommend you use rustup to install or configure such a version.

## Install Rust

Before we begin, we need to install Rust using the [rustup](https://www.rustup.rs/) installer:

```bash
curl https://sh.rustup.rs -sSf | sh
```

If you already have rustup installed, run this command to ensure you have the latest version of Rust:

```bash
rustup update
```

Actix web framework requires rust version 1.21 and up.

## Running Examples

The fastest way to start experimenting with actix web is to clone the actix web repository
and run the included examples in the examples/ directory. The following set of
commands runs the `basics` example:

```bash
git clone https://github.com/actix/actix-web
cd actix-web/examples/basics
cargo run
```

Check [examples/](https://github.com/actix/actix-web/tree/master/examples) directory for more examples.
