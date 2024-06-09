use std::{
    io,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
};

use actix_web::{middleware, web, App, HttpServer};

async fn hello() -> &'static str {
    "Hello world!"
}

#[actix_web::main]
async fn main() -> io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    let core_ids = core_affinity::get_core_ids().unwrap();
    let n_core_ids = core_ids.len();
    let next_core_id = Arc::new(AtomicUsize::new(0));

    HttpServer::new(move || {
        let pin = Arc::clone(&next_core_id).fetch_add(1, Ordering::AcqRel);
        log::info!(
            "setting CPU affinity for worker {}: pinning to core {}",
            thread::current().name().unwrap(),
            pin,
        );
        core_affinity::set_for_current(core_ids[pin]);

        App::new()
            .wrap(middleware::Logger::default())
            .service(web::resource("/").get(hello))
    })
    .bind(("127.0.0.1", 8080))?
    .workers(n_core_ids)
    .run()
    .await
}
