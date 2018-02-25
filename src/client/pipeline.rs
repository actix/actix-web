use std::{io, mem};
use bytes::{Bytes, BytesMut};
use http::header::CONTENT_ENCODING;
use futures::{Async, Future, Poll};
use futures::unsync::oneshot;

use actix::prelude::*;

use error::Error;
use body::{Body, BodyStream};
use context::{Frame, ActorHttpContext};
use headers::ContentEncoding;
use error::PayloadError;
use server::WriterState;
use server::shared::SharedBytes;
use server::encoding::PayloadStream;
use super::{ClientRequest, ClientResponse};
use super::{Connect, Connection, ClientConnector, ClientConnectorError};
use super::HttpClientWriter;
use super::{HttpResponseParser, HttpResponseParserError};

/// A set of errors that can occur during sending request and reading response
#[derive(Fail, Debug)]
pub enum SendRequestError {
    /// Failed to connect to host
    #[fail(display="Failed to connect to host: {}", _0)]
    Connector(#[cause] ClientConnectorError),
    /// Error parsing response
    #[fail(display="{}", _0)]
    ParseError(#[cause] HttpResponseParserError),
    /// Error reading response payload
    #[fail(display="Error reading response payload: {}", _0)]
    Io(#[cause] io::Error),
}

impl From<io::Error> for SendRequestError {
    fn from(err: io::Error) -> SendRequestError {
        SendRequestError::Io(err)
    }
}

enum State {
    New,
    Connect(actix::dev::Request<Unsync, ClientConnector, Connect>),
    Connection(Connection),
    Send(Box<Pipeline>),
    None,
}

/// `SendRequest` is a `Future` which represents asynchronous request sending process.
#[must_use = "SendRequest does nothing unless polled"]
pub struct SendRequest {
    req: ClientRequest,
    state: State,
    conn: Addr<Unsync, ClientConnector>,
}

impl SendRequest {
    pub(crate) fn new(req: ClientRequest) -> SendRequest {
        SendRequest::with_connector(req, ClientConnector::from_registry())
    }

    pub(crate) fn with_connector(req: ClientRequest, conn: Addr<Unsync, ClientConnector>)
                                 -> SendRequest
    {
        SendRequest{
            req: req,
            state: State::New,
            conn: conn}
    }

    pub(crate) fn with_connection(req: ClientRequest, conn: Connection) -> SendRequest
    {
        SendRequest{
            req: req,
            state: State::Connection(conn),
            conn: ClientConnector::from_registry()}
    }
}

impl Future for SendRequest {
    type Item = ClientResponse;
    type Error = SendRequestError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            let state = mem::replace(&mut self.state, State::None);

            match state {
                State::New =>
                    self.state = State::Connect(self.conn.send(Connect(self.req.uri().clone()))),
                State::Connect(mut conn) => match conn.poll() {
                    Ok(Async::NotReady) => {
                        self.state = State::Connect(conn);
                        return Ok(Async::NotReady);
                    },
                    Ok(Async::Ready(result)) => match result {
                        Ok(stream) => {
                            self.state = State::Connection(stream)
                        },
                        Err(err) => return Err(SendRequestError::Connector(err)),
                    },
                    Err(_) => return Err(SendRequestError::Connector(
                        ClientConnectorError::Disconnected))
                },
                State::Connection(stream) => {
                    let mut writer = HttpClientWriter::new(SharedBytes::default());
                    writer.start(&mut self.req)?;

                    let body = match self.req.replace_body(Body::Empty) {
                        Body::Streaming(stream) => IoBody::Payload(stream),
                        Body::Actor(ctx) => IoBody::Actor(ctx),
                        _ => IoBody::Done,
                    };

                    let mut pl = Box::new(Pipeline {
                        body: body,
                        conn: stream,
                        writer: writer,
                        parser: Some(HttpResponseParser::default()),
                        parser_buf: BytesMut::new(),
                        disconnected: false,
                        drain: None,
                        decompress: None,
                        should_decompress: self.req.response_decompress(),
                        write_state: RunningState::Running,
                    });
                    self.state = State::Send(pl);
                },
                State::Send(mut pl) => {
                    pl.poll_write()
                        .map_err(|e| io::Error::new(
                            io::ErrorKind::Other, format!("{}", e).as_str()))?;

                    match pl.parse() {
                        Ok(Async::Ready(mut resp)) => {
                            resp.set_pipeline(pl);
                            return Ok(Async::Ready(resp))
                        },
                        Ok(Async::NotReady) => {
                            self.state = State::Send(pl);
                            return Ok(Async::NotReady)
                        },
                        Err(err) => return Err(SendRequestError::ParseError(err))
                    }
                }
                State::None => unreachable!(),
            }
        }
    }
}


pub(crate) struct Pipeline {
    body: IoBody,
    conn: Connection,
    writer: HttpClientWriter,
    parser: Option<HttpResponseParser>,
    parser_buf: BytesMut,
    disconnected: bool,
    drain: Option<oneshot::Sender<()>>,
    decompress: Option<PayloadStream>,
    should_decompress: bool,
    write_state: RunningState,
}

enum IoBody {
    Payload(BodyStream),
    Actor(Box<ActorHttpContext>),
    Done,
}

#[derive(Debug, PartialEq)]
enum RunningState {
    Running,
    Paused,
    Done,
}

impl RunningState {
    #[inline]
    fn pause(&mut self) {
        if *self != RunningState::Done {
            *self = RunningState::Paused
        }
    }
    #[inline]
    fn resume(&mut self) {
        if *self != RunningState::Done {
            *self = RunningState::Running
        }
    }
}

impl Pipeline {

    #[inline]
    pub fn parse(&mut self) -> Poll<ClientResponse, HttpResponseParserError> {
        match self.parser.as_mut().unwrap().parse(&mut self.conn, &mut self.parser_buf) {
            Ok(Async::Ready(resp)) => {
                // check content-encoding
                if self.should_decompress {
                    if let Some(enc) = resp.headers().get(CONTENT_ENCODING) {
                        if let Ok(enc) = enc.to_str() {
                            match ContentEncoding::from(enc) {
                                ContentEncoding::Auto | ContentEncoding::Identity => (),
                                enc => self.decompress = Some(PayloadStream::new(enc)),
                            }
                        }
                    }
                }

                Ok(Async::Ready(resp))
            }
            val => val,
        }
    }

    #[inline]
    pub fn poll(&mut self) -> Poll<Option<Bytes>, PayloadError> {
        let mut need_run = false;

        // need write?
        match self.poll_write()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{}", e)))?
        {
            Async::NotReady => need_run = true,
            _ => (),
        }

        // need read?
        if self.parser.is_some() {
            loop {
                match self.parser.as_mut().unwrap()
                    .parse_payload(&mut self.conn, &mut self.parser_buf)?
                {
                    Async::Ready(Some(b)) => {
                        if let Some(ref mut decompress) = self.decompress {
                            match decompress.feed_data(b) {
                                Ok(Some(b)) => return Ok(Async::Ready(Some(b))),
                                Ok(None) => return Ok(Async::NotReady),
                                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock =>
                                    continue,
                                Err(err) => return Err(err.into()),
                            }
                        } else {
                            return Ok(Async::Ready(Some(b)))
                        }
                    },
                    Async::Ready(None) => {
                        let _ = self.parser.take();
                        break
                    }
                    Async::NotReady => return Ok(Async::NotReady),
                }
            }
        }

        // eof
        if let Some(mut decompress) = self.decompress.take() {
            let res = decompress.feed_eof();
            if let Some(b) = res? {
                return Ok(Async::Ready(Some(b)))
            }
        }

        if need_run {
            Ok(Async::NotReady)
        } else {
            Ok(Async::Ready(None))
        }
    }

    #[inline]
    pub fn poll_write(&mut self) -> Poll<(), Error> {
        if self.write_state == RunningState::Done {
            return Ok(Async::Ready(()))
        }

        let mut done = false;

        if self.drain.is_none() && self.write_state != RunningState::Paused {
            'outter: loop {
                let result = match mem::replace(&mut self.body, IoBody::Done) {
                    IoBody::Payload(mut body) => {
                        match body.poll()? {
                            Async::Ready(None) => {
                                self.writer.write_eof()?;
                                self.disconnected = true;
                                break
                            },
                            Async::Ready(Some(chunk)) => {
                                self.body = IoBody::Payload(body);
                                self.writer.write(chunk.into())?
                            }
                            Async::NotReady => {
                                done = true;
                                self.body = IoBody::Payload(body);
                                break
                            },
                        }
                    },
                    IoBody::Actor(mut ctx) => {
                        if self.disconnected {
                            ctx.disconnected();
                        }
                        match ctx.poll()? {
                            Async::Ready(Some(vec)) => {
                                if vec.is_empty() {
                                    self.body = IoBody::Actor(ctx);
                                    break
                                }
                                let mut res = None;
                                for frame in vec {
                                    match frame {
                                        Frame::Chunk(None) => {
                                            // info.context = Some(ctx);
                                            self.disconnected = true;
                                            self.writer.write_eof()?;
                                            break 'outter
                                        },
                                        Frame::Chunk(Some(chunk)) =>
                                            res = Some(self.writer.write(chunk)?),
                                        Frame::Drain(fut) => self.drain = Some(fut),
                                    }
                                }
                                self.body = IoBody::Actor(ctx);
                                if self.drain.is_some() {
                                    self.write_state.resume();
                                    break
                                }
                                res.unwrap()
                            },
                            Async::Ready(None) => {
                                done = true;
                                break
                            }
                            Async::NotReady => {
                                done = true;
                                self.body = IoBody::Actor(ctx);
                                break
                            }
                        }
                    },
                    IoBody::Done => {
                        self.disconnected = true;
                        done = true;
                        break
                    }
                };

                match result {
                    WriterState::Pause => {
                        self.write_state.pause();
                        break
                    }
                    WriterState::Done => {
                        self.write_state.resume()
                    },
                }
            }
        }

        // flush io but only if we need to
        match self.writer.poll_completed(&mut self.conn, false) {
            Ok(Async::Ready(_)) => {
                if self.disconnected {
                    self.write_state = RunningState::Done;
                } else {
                    self.write_state.resume();
                }

                // resolve drain futures
                if let Some(tx) = self.drain.take() {
                    let _ = tx.send(());
                }
                // restart io processing
                if !done || self.write_state == RunningState::Done {
                    self.poll_write()
                } else {
                    Ok(Async::NotReady)
                }
            },
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(err) => Err(err.into()),
        }
    }
}
