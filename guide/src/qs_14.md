# Database integration

## Diesel

At the moment, Diesel 1.0 does not support asynchronous operations,
but it possible to use the `actix` synchronous actor system as a database interface api.

Technically, sync actors are worker style actors. Multiple sync actors
can be run in parallel and process messages from same queue. Sync actors work in mpsc mode.

Let's create a simple database api that can insert a new user row into a SQLite table.
We must define a sync actor and a connection that this actor will use. The same approach
can be used for other databases.

```rust,ignore
use actix::prelude::*;

struct DbExecutor(SqliteConnection);

impl Actor for DbExecutor {
    type Context = SyncContext<Self>;
}
```

This is the definition of our actor. Now, we must define the *create user* message and response.

```rust,ignore
struct CreateUser {
    name: String,
}

impl Message for CreateUser {
    type Result = Result<User, Error>;
}
```

We can send a `CreateUser` message to the `DbExecutor` actor, and as a result, we will receive a
`User` model instance. Next, we must define the handler implementation for this message.

```rust,ignore
impl Handler<CreateUser> for DbExecutor {
    type Result = Result<User, Error>;

    fn handle(&mut self, msg: CreateUser, _: &mut Self::Context) -> Self::Result
    {
        use self::schema::users::dsl::*;

        // Create insertion model
        let uuid = format!("{}", uuid::Uuid::new_v4());
        let new_user = models::NewUser {
            id: &uuid,
            name: &msg.name,
        };

        // normal diesel operations
        diesel::insert_into(users)
            .values(&new_user)
            .execute(&self.0)
            .expect("Error inserting person");

        let mut items = users
            .filter(id.eq(&uuid))
            .load::<models::User>(&self.0)
            .expect("Error loading person");

        Ok(items.pop().unwrap())
    }
}
```

That's it! Now, we can use the *DbExecutor* actor from any http handler or middleware.
All we need is to start *DbExecutor* actors and store the address in a state where http handler
can access it.

```rust,ignore
/// This is state where we will store *DbExecutor* address.
struct State {
    db: Addr<Syn, DbExecutor>,
}

fn main() {
    let sys = actix::System::new("diesel-example");

    // Start 3 parallel db executors
    let addr = SyncArbiter::start(3, || {
        DbExecutor(SqliteConnection::establish("test.db").unwrap())
    });

    // Start http server
    HttpServer::new(move || {
        App::with_state(State{db: addr.clone()})
            .resource("/{name}", |r| r.method(Method::GET).a(index))})
        .bind("127.0.0.1:8080").unwrap()
        .start().unwrap();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
```

We will use the address in a request handler. The handle returns a future object;
thus, we receive the message response asynchronously.
`Route::a()` must be used for async handler registration.


```rust,ignore
/// Async handler
fn index(req: HttpRequest<State>) -> Box<Future<Item=HttpResponse, Error=Error>> {
    let name = &req.match_info()["name"];

    // Send message to `DbExecutor` actor
    req.state().db.send(CreateUser{name: name.to_owned()})
        .from_err()
        .and_then(|res| {
            match res {
                Ok(user) => Ok(HttpResponse::Ok().json(user)),
                Err(_) => Ok(HttpResponse::InternalServerError().into())
            }
        })
        .responder()
}
```

> A full example is available in the
> [examples directory](https://github.com/actix/actix-web/tree/master/examples/diesel/).

> More information on sync actors can be found in the
> [actix documentation](https://docs.rs/actix/0.5.0/actix/sync/index.html).
