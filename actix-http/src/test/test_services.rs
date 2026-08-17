use actix_service::{fn_service, Service};
use actix_utils::future::ready;

use crate::{body::MessageBody, Error, Request, Response, StatusCode};

pub(crate) fn ok_service(
) -> impl Service<Request, Response = Response<impl MessageBody>, Error = Error> {
    status_service(StatusCode::OK)
}

fn status_service(
    status: StatusCode,
) -> impl Service<Request, Response = Response<impl MessageBody>, Error = Error> {
    fn_service(move |_req: Request| ready(Ok::<_, Error>(Response::new(status))))
}
