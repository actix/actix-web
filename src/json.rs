use serde_json;
use serde::Serialize;

use error::Error;
use handler::Responder;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

/// Json response helper
///
/// The `Json` type allows you to respond with well-formed JSON data: simply return a value of
/// type Json<T> where T is the type of a structure to serialize into *JSON*. The
/// type `T` must implement the `Serialize` trait from *serde*.
///
/// ```rust
/// # extern crate actix_web;
/// # #[macro_use] extern crate serde_derive;
/// # use actix_web::*;
/// # 
/// #[derive(Serialize)]
/// struct MyObj {
///     name: String,
/// }
///
/// fn index(req: HttpRequest) -> Result<Json<MyObj>> {
///     Ok(Json(MyObj{name: req.match_info().query("name")?}))
/// }
/// # fn main() {}
/// ```
pub struct Json<T: Serialize> (pub T);

impl<T: Serialize> Responder for Json<T> {
    type Item = HttpResponse;
    type Error = Error;

    fn respond_to(self, _: HttpRequest) -> Result<HttpResponse, Error> {
        let body = serde_json::to_string(&self.0)?;

        Ok(HttpResponse::Ok()
           .content_type("application/json")
           .body(body)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{header, Method};
    use application::Application;

    #[derive(Serialize)]
    struct MyObj {
        name: &'static str,
    }

    #[test]
    fn test_json() {
        let json = Json(MyObj{name: "test"});
        let resp = json.respond_to(HttpRequest::default()).unwrap();
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(), "application/json");
    }

}
