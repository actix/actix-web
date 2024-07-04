use actix_utils::future::{ready, Ready};
use actix_web::{dev::Payload, Error, FromRequest, HttpRequest};

use crate::multipart::Multipart;

/// Extract request's payload as multipart stream.
///
/// Content-type: multipart/*;
///
/// # Examples
///
/// ```
/// use actix_web::{web, HttpResponse};
/// use actix_multipart::Multipart;
/// use futures_util::StreamExt as _;
///
/// async fn index(mut payload: Multipart) -> actix_web::Result<HttpResponse> {
///     // iterate over multipart stream
///     while let Some(item) = payload.next().await {
///            let mut field = item?;
///
///            // Field in turn is stream of *Bytes* object
///            while let Some(chunk) = field.next().await {
///                println!("-- CHUNK: \n{:?}", std::str::from_utf8(&chunk?));
///            }
///     }
///
///     Ok(HttpResponse::Ok().finish())
/// }
/// ```
impl FromRequest for Multipart {
    type Error = Error;
    type Future = Ready<Result<Multipart, Error>>;

    #[inline]
    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        ready(Ok(Multipart::from_req(req, payload)))
    }
}
