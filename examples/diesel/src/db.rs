//! Db executor actor
use uuid;
use diesel;
use actix_web::*;
use actix::prelude::*;
use diesel::prelude::*;

use models;
use schema;

/// This is db executor actor. We are going to run 3 of them in parallele.
pub struct DbExecutor(pub SqliteConnection);

/// This is only message that this actor can handle, but it is easy to extend number of
/// messages.
pub struct CreateUser {
    pub name: String,
}

impl ResponseType for CreateUser {
    type Item = models::User;
    type Error = Error;
}

impl Actor for DbExecutor {
    type Context = SyncContext<Self>;
}

impl Handler<CreateUser> for DbExecutor {
    fn handle(&mut self, msg: CreateUser, _: &mut Self::Context)
              -> Response<Self, CreateUser>
    {
        use self::schema::users::dsl::*;

        let uuid = format!("{}", uuid::Uuid::new_v4());
        let new_user = models::NewUser {
            id: &uuid,
            name: &msg.name,
        };

        diesel::insert_into(users)
            .values(&new_user)
            .execute(&self.0)
            .expect("Error inserting person");

        let mut items = users
            .filter(id.eq(&uuid))
            .load::<models::User>(&self.0)
            .expect("Error loading person");

        Self::reply(items.pop().unwrap())
    }
}
