extern crate actix;
extern crate actix_web;
extern crate env_logger;

use actix::*;
use actix_web::*;

struct MyRoute;

impl Actor for MyRoute {
    type Context = HttpContext<Self>;
}

impl Route for MyRoute {
    type State = ();

    fn request(req: HttpRequest, payload: Payload, ctx: &mut HttpContext<Self>) -> Reply<Self> {
        println!("{:?}", req);
        match req.multipart(payload) {
            Ok(multipart) => {
                ctx.add_stream(multipart);
                Reply::async(MyRoute)
            },
            // can not read multipart
            Err(_) => {
                Reply::reply(httpcodes::HTTPBadRequest)
            }
        }
    }
}

impl ResponseType<multipart::MultipartItem> for MyRoute {
    type Item = ();
    type Error = ();
}

impl StreamHandler<multipart::MultipartItem, PayloadError> for MyRoute {
    fn finished(&mut self, ctx: &mut Self::Context) {
        println!("FINISHED");
        ctx.start(httpcodes::HTTPOk);
        ctx.write_eof();
    }
}

impl Handler<multipart::MultipartItem, PayloadError> for MyRoute {
    fn handle(&mut self, msg: multipart::MultipartItem, ctx: &mut HttpContext<Self>)
              -> Response<Self, multipart::MultipartItem>
    {
        println!("==== FIELD ==== {:?}", msg);
        //if let Some(req) = self.req.take() {
        Self::empty()
    }
}

fn main() {
    let _ = env_logger::init();
    let sys = actix::System::new("multipart-example");

    HttpServer::new(
        RoutingMap::default()
            .app("/", Application::default()
                 .resource("/multipart", |r| {
                     r.post::<MyRoute>();
                 })
                 .finish())
            .finish())
        .serve::<_, ()>("127.0.0.1:8080").unwrap();

    let _ = sys.run();
}
