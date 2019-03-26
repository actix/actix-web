use actix_http::body::Body;
use actix_http::client::{ClientResponse, ConnectError, Connection, SendRequestError};
use actix_http::{http, RequestHead};
use actix_service::Service;
use futures::Future;

pub(crate) struct ConnectorWrapper<T>(pub T);

pub(crate) trait Connect {
    fn send_request(
        &mut self,
        head: RequestHead,
        body: Body,
    ) -> Box<Future<Item = ClientResponse, Error = SendRequestError>>;
}

impl<T> Connect for ConnectorWrapper<T>
where
    T: Service<Request = http::Uri, Error = ConnectError>,
    T::Response: Connection,
    <T::Response as Connection>::Future: 'static,
    T::Future: 'static,
{
    fn send_request(
        &mut self,
        head: RequestHead,
        body: Body,
    ) -> Box<Future<Item = ClientResponse, Error = SendRequestError>> {
        Box::new(
            self.0
                // connect to the host
                .call(head.uri.clone())
                .from_err()
                // send request
                .and_then(move |connection| connection.send_request(head, body)),
        )
    }
}
