//! Sync version of client's HTTP primitives
extern crate serde;
extern crate futures;
extern crate bytes;
extern crate tokio;
extern crate http;
extern crate encoding;
extern crate mime;
extern crate serde_urlencoded;
extern crate serde_json;
extern crate crossbeam_channel;

use std::rc::Rc;
use std::io;
use std::thread;
use std::ops::{Deref, DerefMut};

use self::mime::Mime;
use self::encoding::all::UTF_8;
use self::encoding::label::encoding_from_whatwg_label;
use self::encoding::types::{DecoderTrap, Encoding};
use self::encoding::EncodingRef;
use self::http::{header, HeaderMap};
use self::bytes::Bytes;
use self::serde::de::DeserializeOwned;
use self::futures::{Future, Stream};
use self::futures::sync::{oneshot};

use super::response::ClientMessage;
use super::pipeline::Pipeline;
use ::httpmessage::HttpMessage;
use ::error::{JsonPayloadError, PayloadError, UrlencodedError, ContentTypeError};
use ::dev::{JsonBody, MessageBody, UrlEncoded};
use super::SendRequestError;
mod async {
    pub use super::super::request::ClientRequest;
    pub use super::super::response::ClientResponse;
}

/// An synchronous adaptor for body
pub trait SyncBody<T, E>: Future<Item=T, Error=E> {
    /// Reads entire body synchronously.
    ///
    /// Note: You should not attempt to read the whole future on the same thread
    /// as actix event loop.
    fn collect(self) -> Result<T, E>  where Self: Sized {
        self.wait()
    }
}

impl<T> SyncBody<Bytes, PayloadError> for MessageBody<T>
    where T: HttpMessage + Stream<Item=Bytes, Error=PayloadError> + 'static {}

impl<T, U: DeserializeOwned + 'static> SyncBody<U, UrlencodedError> for UrlEncoded<T, U>
    where T: HttpMessage + Stream<Item=Bytes, Error=PayloadError> + 'static {}

impl<T, U: DeserializeOwned + 'static> SyncBody<U, JsonPayloadError> for JsonBody<T, U>
    where T: HttpMessage + Stream<Item=Bytes, Error=PayloadError> + 'static {}

/// An synchronous HTTP Client Request
pub struct ClientRequest(pub async::ClientRequest);

impl Deref for ClientRequest {
    type Target = async::ClientRequest;

    fn deref(&self) -> &async::ClientRequest {
        &self.0
    }
}

impl DerefMut for ClientRequest {
    fn deref_mut(&mut self) -> &mut async::ClientRequest {
        &mut self.0
    }
}

/// An synchronous HTTP Client Response
pub struct ClientResponse {
    sender: ClientSender,
    message: ClientMessage,
    pipeline: Option<Box<Pipeline>>,
}

impl ClientResponse {
    fn new(message: ClientMessage, pipeline: Option<Box<Pipeline>>, sender: ClientSender) -> Self {
        Self {
            sender,
            message,
            pipeline
        }
    }

    ///Transforms self into asynchronous response
    pub fn into_async(self) -> async::ClientResponse {
        async::ClientResponse::from_parts(self.message, self.pipeline)
    }

    /// Retrieves headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.message.headers
    }

    /// Read the request content type. If request does not contain
    /// *Content-Type* header, empty str get returned.
    pub fn content_type(&self) -> &str {
        if let Some(content_type) = self.headers().get(header::CONTENT_TYPE) {
            if let Ok(content_type) = content_type.to_str() {
                return content_type.split(';').next().unwrap().trim();
            }
        }
        ""
    }

    /// Convert the request content type to a known mime type.
    fn mime_type(&self) -> Result<Option<Mime>, ContentTypeError> {
        if let Some(content_type) = self.headers().get(header::CONTENT_TYPE) {
            if let Ok(content_type) = content_type.to_str() {
                return match content_type.parse() {
                    Ok(mt) => Ok(Some(mt)),
                    Err(_) => Err(ContentTypeError::ParseError),
                };
            } else {
                return Err(ContentTypeError::ParseError);
            }
        }
        Ok(None)
    }

    /// Get content type encoding
    ///
    /// UTF-8 is used by default, If request charset is not set.
    fn encoding(&self) -> Result<EncodingRef, ContentTypeError> {
        if let Some(mime_type) = self.mime_type()? {
            if let Some(charset) = mime_type.get_param("charset") {
                if let Some(enc) = encoding_from_whatwg_label(charset.as_str()) {
                    Ok(enc)
                } else {
                    Err(ContentTypeError::UnknownEncoding)
                }
            } else {
                Ok(UTF_8)
            }
        } else {
            Ok(UTF_8)
        }
    }

    ///Synchronously receive response body as raw bytes.
    pub fn sync_body(self) -> Result<Bytes, PayloadError> {
        let (sender, receiver) = oneshot::channel();
        self.sender.send((SyncJob::CollectBody(self.message, self.pipeline), sender));

        match receiver.wait() {
            Ok(rsp) => match rsp {
                SyncJobResult::Body(result) => result,
                _ => unreachable!()
            },
            Err(_canceled) => panic!("worker thread panicked!"),
        }
    }

    ///Synchronously receive response body as url encoded form
    pub fn sync_urlencoded<T: DeserializeOwned>(self) -> Result<T, UrlencodedError> {
        // check content type
        if self.content_type().to_lowercase() != "application/x-www-form-urlencoded" {
            return Err(UrlencodedError::ContentType);
        }
        let encoding = self.encoding().map_err(|_| UrlencodedError::ContentType)?;

        let body = self.sync_body().map_err(|error| UrlencodedError::from(error))?;

        let enc: *const Encoding = encoding as *const Encoding;
        if enc == UTF_8 {
            serde_urlencoded::from_bytes::<T>(&body).map_err(|_| UrlencodedError::Parse)
        } else {
            let body = encoding .decode(&body, DecoderTrap::Strict) .map_err(|_| UrlencodedError::Parse)?;
            serde_urlencoded::from_str::<T>(&body).map_err(|_| UrlencodedError::Parse)
        }
    }

    ///Synchronously receive response body as json.
    pub fn sync_json<T: DeserializeOwned>(self) -> Result<T, JsonPayloadError> {
        let json = match self.mime_type() {
            Ok(Some(mime)) => mime.subtype() == mime::JSON || mime.suffix() == Some(mime::JSON),
            _ => false
        };

        if !json {
            return Err(JsonPayloadError::ContentType);
        }

        let body = self.sync_body().map_err(|error| JsonPayloadError::from(error))?;
        serde_json::from_slice::<T>(&body).map_err(|error| JsonPayloadError::from(error))
    }
}

enum SyncJob {
    Request(async::ClientRequest),
    CollectBody(ClientMessage, Option<Box<Pipeline>>),
}

enum SyncJobResult {
    Response(ClientMessage, Option<Box<Pipeline>>),
    SendError(SendRequestError),
    Body(Result<Bytes, PayloadError>),
}

type ClientSender = crossbeam_channel::Sender<(SyncJob, oneshot::Sender<SyncJobResult>)>;
///Synchronous HTTP Client
pub struct Client {
    sender: ClientSender,
    _worker: thread::JoinHandle<()>
}

impl Client {
    ///Creates new instance.
    pub fn new() -> io::Result<Self> {
        let (sender, receiver) = crossbeam_channel::unbounded();
        let worker = thread::Builder::new().name("actix-web-sync-worker".into()).spawn(move || {
            while let Some((req, receiver)) = receiver.recv() {
                let receiver: oneshot::Sender<SyncJobResult> = receiver;
                let _ = match req {
                    SyncJob::Request(req) => match req.send().wait() {
                        Ok(rsp) => {
                            let (msg, pipeline) = rsp.into_parts();
                            //TODO: Consider Do we need Rc actually?
                            let msg = match Rc::try_unwrap(msg) {
                                Ok(msg) => msg.into_inner(),
                                Err(_) => panic!("Unable to unwrap")
                            };
                            receiver.send(SyncJobResult::Response(msg, pipeline))
                        },
                        Err(error) => receiver.send(SyncJobResult::SendError(error))
                    },
                    SyncJob::CollectBody(message, pipeline) => {
                        let body = async::ClientResponse::from_parts(message, pipeline).body();
                        receiver.send(SyncJobResult::Body(body.collect()))
                    },
                };
            }
        })?;

        Ok(Self {
            sender: sender,
            _worker: worker
        })
    }

    ///Sends HTTP request synchronously.
    pub fn send(&self, request: async::ClientRequest) -> Result<ClientResponse, SendRequestError> {
        let (sender, receiver) = oneshot::channel();
        self.sender.send((SyncJob::Request(request), sender));

        match receiver.wait() {
            Ok(rsp) => match rsp {
                SyncJobResult::Response(message, pipeline) => Ok(ClientResponse::new(message, pipeline, self.sender.clone())),
                SyncJobResult::SendError(error) => Err(error),
                _ => unreachable!()
            },
            Err(_canceled) => panic!("worker thread panicked!"),
        }
    }
}
