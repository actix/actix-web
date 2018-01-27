use actix_web::*;
use futures::Future;


#[derive(Deserialize,Serialize, Debug)]
struct Info {
    username: String,
    email: String,
    password: String,
    confirm_password: String,
}

pub fn info(req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    req.json()                   
        .from_err()
        .and_then(|res: Info| {
            Ok(httpcodes::HTTPOk.build().json(res)?)
        }).responder()
}
