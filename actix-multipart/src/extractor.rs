//! Multipart payload support
use actix_web::{dev::Payload, Error, FromRequest, HttpRequest};

use crate::server::Multipart;

/// Get request's payload as multipart stream
///
/// Content-type: multipart/form-data;
///
/// ## Server example
///
/// ```rust
/// # use futures::{Future, Stream};
/// # use futures::future::{ok, result, Either};
/// use actix_web::{web, HttpResponse, Error};
/// use actix_multipart as mp;
///
/// fn index(payload: mp::Multipart) -> impl Future<Item = HttpResponse, Error = Error> {
///     payload.from_err()               // <- get multipart stream for current request
///        .and_then(|field| {           // <- iterate over multipart items
///            // Field in turn is stream of *Bytes* object
///            field.from_err()
///                .fold((), |_, chunk| {
///                    println!("-- CHUNK: \n{:?}", std::str::from_utf8(&chunk));
///                        Ok::<_, Error>(())
///                    })
///         })
///         .fold((), |_, _| Ok::<_, Error>(()))
///         .map(|_| HttpResponse::Ok().into())
/// }
/// # fn main() {}
/// ```
impl FromRequest for Multipart {
    type Error = Error;
    type Future = Result<Multipart, Error>;
    type Config = ();

    #[inline]
    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        Ok(Multipart::new(req.headers(), payload.take()))
    }
}
