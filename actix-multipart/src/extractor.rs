#[expect(deprecated, reason = "preserve the public ready future type")]
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
#[expect(deprecated, reason = "preserve the public ready future type")]
impl FromRequest for Multipart {
    type Error = Error;
    type Future = Ready<Result<Multipart, Error>>;

    #[inline]
    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        ready(Ok(Multipart::from_req(req, payload)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[expect(
        deprecated,
        reason = "check compatibility with the public ready future type"
    )]
    fn extractor_future_type() {
        let req = actix_web::test::TestRequest::default().to_http_request();

        let _: actix_utils::future::Ready<Result<Multipart, Error>> =
            Multipart::from_request(&req, &mut Payload::None);
    }
}
