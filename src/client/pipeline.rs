use std::{io, mem};
use bytes::{Bytes, BytesMut};
use futures::{Async, Future, Poll};
use futures::unsync::oneshot;

use actix::prelude::*;

use error::Error;
use body::{Body, BodyStream};
use context::{Frame, ActorHttpContext};
use error::PayloadError;
use server::WriterState;
use server::shared::SharedBytes;
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
                                parser: HttpResponseParser::default(),
                                parser_buf: BytesMut::new(),
                                disconnected: false,
                                running: RunningState::Running,
                                drain: None,
                            });
                            self.state = State::Send(pl);
                        },
                        Err(err) => return Err(SendRequestError::Connector(err)),
                    },
                    Err(_) =>
                        return Err(SendRequestError::Connector(ClientConnectorError::Disconnected))
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
    parser: HttpResponseParser,
    parser_buf: BytesMut,
    disconnected: bool,
    running: RunningState,
    drain: Option<oneshot::Sender<()>>,
}

enum IoBody {
    Payload(BodyStream),
    Actor(Box<ActorHttpContext>),
    Done,
}

#[derive(PartialEq)]
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
        self.parser.parse(&mut self.conn, &mut self.parser_buf)
    }

    #[inline]
    pub fn poll(&mut self) -> Poll<Option<Bytes>, PayloadError> {
        self.poll_write()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{}", e).as_str()))?;
        Ok(self.parser.parse_payload(&mut self.conn, &mut self.parser_buf)?)
    }

    #[inline]
    pub fn poll_write(&mut self) -> Poll<(), Error> {
        if self.running == RunningState::Done {
            return Ok(Async::Ready(()))
        }

        let mut done = false;

        if self.drain.is_none() && self.running != RunningState::Paused {
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
                                    self.running.resume();
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
                        done = true;
                        break
                    }
                };

                match result {
                    WriterState::Pause => {
                        self.running.pause();
                        break
                    }
                    WriterState::Done => {
                        self.running.resume()
                    },
                }
            }
        }

        // flush io but only if we need to
        match self.writer.poll_completed(&mut self.conn, false) {
            Ok(Async::Ready(_)) => {
                self.running.resume();

                // resolve drain futures
                if let Some(tx) = self.drain.take() {
                    let _ = tx.send(());
                }
                // restart io processing
                if !done {
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
