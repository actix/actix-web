//! Db executor actor
use std::io;
use uuid;
use actix_web::*;
use actix::prelude::*;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;


/// This is db executor actor. We are going to run 3 of them in parallel.
pub struct DbExecutor(pub Pool<SqliteConnectionManager>);

/// This is only message that this actor can handle, but it is easy to extend number of
/// messages.
pub struct CreateUser {
    pub name: String,
}

impl Message for CreateUser {
    type Result = Result<String, io::Error>;
}

impl Actor for DbExecutor {
    type Context = SyncContext<Self>;
}

impl Handler<CreateUser> for DbExecutor {
    type Result = Result<String, io::Error>;

    fn handle(&mut self, msg: CreateUser, _: &mut Self::Context) -> Self::Result {
        let conn = self.0.get().unwrap();

        let uuid = format!("{}", uuid::Uuid::new_v4());
        conn.execute("INSERT INTO users (id, name) VALUES ($1, $2)",
                     &[&uuid, &msg.name]).unwrap();

        Ok(conn.query_row("SELECT name FROM users WHERE id=$1", &[&uuid], |row| {
            row.get(0)
        }).map_err(|_| io::Error::new(io::ErrorKind::Other, "db error"))?)
    }
}
