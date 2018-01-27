use actix::*;
use actix_web::*;
use futures::future::Future;

#[derive(Deserialize,Serialize, Debug)]
struct Info {
    username: String,
    email: String,
    password: String,
    confirm_password: String,
}
pub fn info(mut req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    req.json()                   
       .from_err()
       .and_then(|res: Info| {  
            Ok(httpcodes::HTTPOk.build().json(res)?)
       }).responder()
}

