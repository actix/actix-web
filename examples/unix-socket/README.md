## Unix domain socket example

```bash
$ curl --unix-socket /tmp/actix-uds.socket http://localhost/
Hello world!
```

Although this will only one thread for handling incoming connections 
according to the 
[documentation](https://actix.github.io/actix-web/actix_web/struct.HttpServer.html#method.start_incoming).

And it does not delete the socket file (`/tmp/actix-uds.socket`) when stopping
the server so it will fail to start next time you run it unless you delete
the socket file manually.
