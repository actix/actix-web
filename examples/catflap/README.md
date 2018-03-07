# catflap

There is the utility [cargo-watch](https://github.com/passcod/cargo-watch) which rebuilds your project when it notices
there are changed files. This is very useful when writing web server and you want to have a fast edit, compile, run
cycle.  

The problem is that when you're using this for a web server the socket that the server uses might still be in use
from the previous run. This makes `cargo-watch` crash. The solution, from the author of `cargo-watch` is `catflap`. It's
another utility that takes ownership of sockets and passes them the program you're running with cargo watch.   

To be able to use the sockets provided by `catflap` you need to read the file descriptor from the environment variable
`LISTEN_FD`.

This example will how you how to do that on a hello world.

By default the server will be running on port `5000`, unless you pass the flag `-p 8080` to catflap. Then it will run on
port `8080`

## Usage
```bash
cd actix-web/examples/catflap
catflap -- cargo watch -x run
# Started http server: 127.0.0.1:8080
```
