use std::cell::UnsafeCell;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering, ATOMIC_BOOL_INIT};
use std::sync::{mpsc, Arc};
use std::{mem, net};

use bytes::{BufMut, Bytes, BytesMut};
use futures::task::Task;
use futures::{Async, Poll};
use mio;
use mio::event::Evented;
use mio::net::TcpStream;
use slab::Slab;

use server::h1decoder::{Decoder, DecoderError, Message};
use server::helpers::SharedMessagePool;

mod channel;
pub(crate) use self::channel::{IoChannel, IoCommand, TaskChannel, TaskCommand};

const TOKEN_NOTIFY: usize = 0;
const TOKEN_START: usize = 2;
const LW_BUFFER_SIZE: usize = 4096;
const HW_BUFFER_SIZE: usize = 32_768;

pub(crate) struct Core {
    mio: mio::Poll,
    events: mio::Events,
    state: Arc<AtomicBool>,
    io: Slab<Io>,
    channel: TaskChannel,
    pool: Arc<SharedMessagePool>,
}

impl Core {
    pub fn new(channel: TaskChannel) -> io::Result<Core> {
        let mio = mio::Poll::new()?;
        let notify = mio::Token(TOKEN_NOTIFY);

        // notify stream
        mio.register(
            channel.registration(),
            notify,
            mio::Ready::readable(),
            mio::PollOpt::edge(),
        )?;

        Ok(Core {
            mio,
            channel,
            io: Slab::new(),
            events: mio::Events::with_capacity(1024),
            state: Arc::new(AtomicBool::new(true)),
            pool: Arc::new(SharedMessagePool::new()),
        })
    }

    pub fn run(mut self) {
        loop {
            self.dispatch_commands();
            self.poll_io();
        }
    }

    fn poll_io(&mut self) {
        //println!("POLL IO");
        // Block waiting for an event to happen
        let _amt = match self.mio.poll(&mut self.events, None) {
            Ok(a) => a,
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => return,
            Err(e) => panic!("Poll error: {}", e),
        };

        let mut modified = false;
        for event in self.events.iter() {
            let token = usize::from(event.token());
            //println!("event: {:?}", event);

            if token != TOKEN_NOTIFY {
                let mut remove = false;
                if let Some(io) = self.io.get_mut(token - TOKEN_START) {
                    match io.poll(event.readiness(), &self.channel) {
                        IoResult::Notify => modified = true,
                        IoResult::Remove => remove = true,
                        IoResult::StopReading => {}
                        IoResult::StopWriting => {}
                        IoResult::NotReady => (),
                    }
                }
                if remove {
                    if self.io.contains(token - TOKEN_START) {
                        let _ = self.io.remove(token - TOKEN_START);
                    }
                }
            }
        }

        if modified {
            self.channel.notify();
        }
    }

    fn dispatch_commands(&mut self) {
        self.channel.start();
        loop {
            match self.channel.try_recv() {
                Ok(IoCommand::AddSource(source, peer)) => {
                    match TcpStream::from_stream(source) {
                        Ok(stream) => match self.add_source(stream, peer) {
                            Ok(token) => (),
                            Err(_) => (),
                        },
                        Err(e) => {
                            error!("Can not register io object: {}", e);
                        }
                    }
                }
                Ok(IoCommand::Bytes(token, bytes)) => {
                    if let Some(io) = self.io.get_mut(token.token) {
                        io.write(bytes);
                    }
                }
                Ok(IoCommand::Drain(token)) => {}
                Ok(IoCommand::Pause(token)) => {}
                Ok(IoCommand::Resume(token)) => {
                    if let Some(io) = self.io.get_mut(token.token) {
                        // io.as_mut().ready = true;
                    }
                }
                Ok(IoCommand::Done {
                    token,
                    graceful,
                }) => {
                    if self.io.contains(token.token) {
                        let _ = self.io.remove(token.token);
                    }
                }
                Err(_) => break,
            }
        }
        self.channel.end();
    }

    fn add_source(
        &mut self, io: TcpStream, peer: Option<net::SocketAddr>,
    ) -> io::Result<IoToken> {
        debug!("adding a new I/O source");
        if self.io.len() == self.io.capacity() {
            let amt = self.io.len();
            self.io.reserve_exact(amt);
        }
        let entry = self.io.vacant_entry();
        let token = entry.key();

        self.mio.register(
            &io,
            mio::Token(TOKEN_START + token),
            mio::Ready::readable() | mio::Ready::writable(),
            mio::PollOpt::edge(),
        )?;

        let token = IoToken {
            token,
        };
        let decoder = Decoder::new(Arc::clone(&self.pool));
        let io = Io {
            buf: BytesMut::with_capacity(HW_BUFFER_SIZE),
            inner: Arc::new(UnsafeCell::new(Inner {
                io,
                token,
                decoder,
                peer,
                lock: ATOMIC_BOOL_INIT,
                task: None,
                ready: false,
                started: false,
                buf: BytesMut::with_capacity(HW_BUFFER_SIZE),
                messages: VecDeque::new(),
            })),
        };

        entry.insert(io);
        Ok(token)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct IoToken {
    token: usize,
}

#[derive(Debug)]
enum IoResult {
    NotReady,
    Notify,
    Remove,
    StopReading,
    StopWriting,
}

struct Io {
    inner: Arc<UnsafeCell<Inner>>,
    buf: BytesMut,
}

impl Io {
    #[inline]
    fn as_mut(&mut self) -> &mut Inner {
        unsafe { &mut *self.inner.as_ref().get() }
    }

    fn write(&mut self, data: Bytes) {
        //self.buf.extend_from_slice(&data);

        let inner: &mut Inner = unsafe { &mut *self.inner.as_ref().get() };

        //while !self.buf.is_empty() {
        match inner.io.write(&data) {
                Ok(0) => {
                    // self.disconnected();
                    // return Err(io::Error::new(io::ErrorKind::WriteZero, ""));
                    return
                }
                Ok(n) => {
                    //self.buf.split_to(n);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    return
                }
                Err(err) => return // Err(err),
            }
        //}
    }

    fn poll(&mut self, ready: mio::Ready, channel: &TaskChannel) -> IoResult {
        let inner: &mut Inner = unsafe { mem::transmute(self.as_mut()) };
        let mut updated = IoResult::NotReady;
        let (read, eof) = match inner.read_from_io() {
            Ok(Async::Ready((n, eof))) => (n, eof),
            Ok(Async::NotReady) => return IoResult::NotReady,
            Err(e) => {
                if inner.started {
                    info!("error during io: {:?}", e);
                    inner.send(Err(e.into()));
                    return IoResult::NotReady;
                } else {
                    info!("error during io before message: {:?}", e);
                    // first message is not ready, so we can drop connection
                    return IoResult::Remove;
                }
            }
        };
        loop {
            let msg = match inner.decoder.decode(&mut inner.buf) {
                Ok(Async::NotReady) => {
                    if eof {
                        if inner.started {
                            inner.send(Ok(Message::Hup));
                        } else {
                            return IoResult::Remove;
                        }
                    }
                    break;
                }
                Ok(Async::Ready(msg)) => Ok(msg),
                Err(e) => Err(e),
            };

            if inner.started {
                inner.send(msg);
            } else {
                if msg.is_ok() {
                    inner.started = true;
                    inner.messages.push_back(msg);
                    let inner = self.inner.clone();
                    let _ = channel.send(TaskCommand::Stream(IoStream {
                        inner,
                    }));
                } else {
                    // first message is not ready, so we can drop connection
                    return IoResult::Remove;
                }
            }
        }
        //println!("READY {:?} {:?}", ready, updated);
        updated
    }
}

pub(crate) struct IoStream {
    inner: Arc<UnsafeCell<Inner>>,
}

impl IoStream {
    pub fn token(&self) -> IoToken {
        self.as_mut().token
    }

    pub fn peer(&self) -> Option<net::SocketAddr> {
        self.as_mut().peer
    }

    pub fn set_notify(&self, task: Task) {
        let inner = self.as_mut();
        while inner.lock.compare_and_swap(false, true, Ordering::Acquire) != false {}
        inner.task = Some(task);
        inner.lock.store(false, Ordering::Release);
    }

    #[inline]
    fn as_mut(&self) -> &mut Inner {
        unsafe { &mut *self.inner.as_ref().get() }
    }

    pub fn try_recv(&self) -> Option<Result<Message, DecoderError>> {
        let inner = self.as_mut();
        while inner.lock.compare_and_swap(false, true, Ordering::Acquire) != false {}
        let result = inner.messages.pop_front();

        inner.lock.store(false, Ordering::Release);
        result
    }
}

struct Inner {
    lock: AtomicBool,
    token: IoToken,
    io: TcpStream,
    decoder: Decoder,
    buf: BytesMut,
    task: Option<Task>,
    peer: Option<net::SocketAddr>,
    ready: bool,
    started: bool,
    messages: VecDeque<Result<Message, DecoderError>>,
}

impl Inner {
    fn send(&mut self, msg: Result<Message, DecoderError>) {
        while self.lock.compare_and_swap(false, true, Ordering::Acquire) != false {}
        self.messages.push_back(msg);

        if let Some(ref task) = self.task.as_ref() {
            task.notify()
        }

        self.lock.store(false, Ordering::Release);
    }

    fn read_from_io(&mut self) -> Poll<(usize, bool), io::Error> {
        let mut read = 0;
        loop {
            unsafe {
                if self.buf.remaining_mut() < LW_BUFFER_SIZE {
                    self.buf.reserve(HW_BUFFER_SIZE);
                }
                match self.io.read(self.buf.bytes_mut()) {
                    Ok(n) => {
                        read += n;
                        if n == 0 {
                            return Ok(Async::Ready((read, true)));
                        } else {
                            self.buf.advance_mut(n);
                        }
                    }
                    Err(e) => {
                        return if e.kind() == io::ErrorKind::WouldBlock {
                            if read != 0 {
                                Ok(Async::Ready((read, false)))
                            } else {
                                Ok(Async::NotReady)
                            }
                        } else {
                            Err(e)
                        };
                    }
                }
            }
        }
    }
}
