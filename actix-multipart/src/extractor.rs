use std::future::{ready, Future};

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

    #[inline]
    fn from_request(
        req: &HttpRequest,
        payload: &mut Payload,
    ) -> impl Future<Output = Result<Self, Self::Error>> + use<> {
        ready(Ok(Multipart::from_req(req, payload)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extractor_is_ready_without_borrowing_request_parts() {
        use std::{
            pin::pin,
            task::{Context, Waker},
        };

        let req = actix_web::test::TestRequest::default().to_http_request();
        let mut payload = Payload::None;
        let future = Multipart::from_request(&req, &mut payload);

        drop(req);
        drop(payload);

        let mut future = pin!(future);
        let mut cx = Context::from_waker(Waker::noop());

        assert!(future.as_mut().poll(&mut cx).is_ready());
    }
}
