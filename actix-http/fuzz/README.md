# actix-http fuzz targets

[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) targets for the HTTP/1 and WebSocket codecs.

- `h1_request`: server-side HTTP/1 request decoder (`h1::Codec`)
- `h1_response`: client-side HTTP/1 response decoder (`h1::ClientCodec` and `ClientPayloadCodec`)
- `ws_frame`: WebSocket frame decoder (`ws::Codec`, server and client mode)

Each target feeds the input in chunks of varying size, so partial reads get exercised too.

```sh
cargo install cargo-fuzz
cd actix-http
cargo +nightly fuzz run h1_request
```
