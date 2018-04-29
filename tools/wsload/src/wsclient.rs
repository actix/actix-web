//! Simple websocket client.

#![allow(unused_variables)]
extern crate actix;
extern crate actix_web;
extern crate clap;
extern crate env_logger;
extern crate futures;
extern crate num_cpus;
extern crate rand;
extern crate time;
extern crate tokio_core;
extern crate url;

use futures::Future;
use rand::{thread_rng, Rng};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use actix::prelude::*;
use actix_web::ws;

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    let _ = env_logger::init();

    let matches = clap::App::new("ws tool")
        .version("0.1")
        .about("Applies load to websocket server")
        .args_from_usage(
            "<url> 'WebSocket url'
                [bin]... -b, 'use binary frames'
                -s, --size=[NUMBER] 'size of PUBLISH packet payload to send in KB'
                -w, --warm-up=[SECONDS] 'seconds before counter values are considered for reporting'
                -r, --sample-rate=[SECONDS] 'seconds between average reports'
                -c, --concurrency=[NUMBER] 'number of websocket connections to open and use concurrently for sending'
                -t, --threads=[NUMBER] 'number of threads to use'
                --max-payload=[NUMBER] 'max size of payload before reconnect KB'",
        )
        .get_matches();

    let bin: bool = matches.value_of("bin").is_some();
    let ws_url = matches.value_of("url").unwrap().to_owned();
    let _ = url::Url::parse(&ws_url).map_err(|e| {
        println!("Invalid url: {}", ws_url);
        std::process::exit(0);
    });

    let threads = parse_u64_default(matches.value_of("threads"), num_cpus::get() as u64);
    let concurrency = parse_u64_default(matches.value_of("concurrency"), 1);
    let payload_size: usize = match matches.value_of("size") {
        Some(s) => parse_u64_default(Some(s), 1) as usize * 1024,
        None => 1024,
    };
    let max_payload_size: usize = match matches.value_of("max-payload") {
        Some(s) => parse_u64_default(Some(s), 0) as usize * 1024,
        None => 0,
    };
    let warmup_seconds = parse_u64_default(matches.value_of("warm-up"), 2) as u64;
    let sample_rate = parse_u64_default(matches.value_of("sample-rate"), 1) as usize;

    let perf_counters = Arc::new(PerfCounters::new());
    let payload =
        Arc::new(thread_rng().gen_ascii_chars().take(payload_size).collect::<String>());

    let sys = actix::System::new("ws-client");

    let _: () = Perf {
        counters: perf_counters.clone(),
        payload: payload.len(),
        sample_rate_secs: sample_rate,
    }.start();

    for t in 0..threads {
        let pl = payload.clone();
        let ws = ws_url.clone();
        let perf = perf_counters.clone();
        let addr = Arbiter::new(format!("test {}", t));

        addr.do_send(actix::msgs::Execute::new(move || -> Result<(), ()> {
            for _ in 0..concurrency {
                let pl2 = pl.clone();
                let perf2 = perf.clone();
                let ws2 = ws.clone();

                Arbiter::handle().spawn(
                    ws::Client::new(&ws)
                        .write_buffer_capacity(0)
                        .connect()
                        .map_err(|e| {
                            println!("Error: {}", e);
                            //Arbiter::system().do_send(actix::msgs::SystemExit(0));
                            ()
                        })
                        .map(move |(reader, writer)| {
                            let addr: Addr<Syn, _> = ChatClient::create(move |ctx| {
                                ChatClient::add_stream(reader, ctx);
                                ChatClient {
                                    url: ws2,
                                    conn: writer,
                                    payload: pl2,
                                    bin: bin,
                                    ts: time::precise_time_ns(),
                                    perf_counters: perf2,
                                    sent: 0,
                                    max_payload_size: max_payload_size,
                                }
                            });
                        }),
                );
            }
            Ok(())
        }));
    }

    let res = sys.run();
}

fn parse_u64_default(input: Option<&str>, default: u64) -> u64 {
    input
        .map(|v| v.parse().expect(&format!("not a valid number: {}", v)))
        .unwrap_or(default)
}

struct Perf {
    counters: Arc<PerfCounters>,
    payload: usize,
    sample_rate_secs: usize,
}

impl Actor for Perf {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Context<Self>) {
        self.sample_rate(ctx);
    }
}

impl Perf {
    fn sample_rate(&self, ctx: &mut Context<Self>) {
        ctx.run_later(Duration::new(self.sample_rate_secs as u64, 0), |act, ctx| {
            let req_count = act.counters.pull_request_count();
            if req_count != 0 {
                let conns = act.counters.pull_connections_count();
                let latency = act.counters.pull_latency_ns();
                let latency_max = act.counters.pull_latency_max_ns();
                println!(
                        "rate: {}, conns: {}, throughput: {:?} kb, latency: {}, latency max: {}",
                        req_count / act.sample_rate_secs,
                        conns / act.sample_rate_secs,
                        (((req_count * act.payload) as f64) / 1024.0)
                            / act.sample_rate_secs as f64,
                        time::Duration::nanoseconds((latency / req_count as u64) as i64),
                        time::Duration::nanoseconds(latency_max as i64)
                    );
            }

            act.sample_rate(ctx);
        });
    }
}

struct ChatClient {
    url: String,
    conn: ws::ClientWriter,
    payload: Arc<String>,
    ts: u64,
    bin: bool,
    perf_counters: Arc<PerfCounters>,
    sent: usize,
    max_payload_size: usize,
}

impl Actor for ChatClient {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Context<Self>) {
        self.send_text();
        self.perf_counters.register_connection();
    }
}

impl ChatClient {
    fn send_text(&mut self) -> bool {
        self.sent += self.payload.len();

        if self.max_payload_size > 0 && self.sent > self.max_payload_size {
            let ws = self.url.clone();
            let pl = self.payload.clone();
            let bin = self.bin;
            let perf_counters = self.perf_counters.clone();
            let max_payload_size = self.max_payload_size;

            Arbiter::handle().spawn(
                ws::Client::new(&self.url)
                    .connect()
                    .map_err(|e| {
                        println!("Error: {}", e);
                        Arbiter::system().do_send(actix::msgs::SystemExit(0));
                        ()
                    })
                    .map(move |(reader, writer)| {
                        let addr: Addr<Syn, _> = ChatClient::create(move |ctx| {
                            ChatClient::add_stream(reader, ctx);
                            ChatClient {
                                url: ws,
                                conn: writer,
                                payload: pl,
                                bin: bin,
                                ts: time::precise_time_ns(),
                                perf_counters: perf_counters,
                                sent: 0,
                                max_payload_size: max_payload_size,
                            }
                        });
                    }),
            );
            false
        } else {
            self.ts = time::precise_time_ns();
            if self.bin {
                self.conn.binary(&self.payload);
            } else {
                self.conn.text(&self.payload);
            }
            true
        }
    }
}

/// Handle server websocket messages
impl StreamHandler<ws::Message, ws::ProtocolError> for ChatClient {
    fn finished(&mut self, ctx: &mut Context<Self>) {
        ctx.stop()
    }

    fn handle(&mut self, msg: ws::Message, ctx: &mut Context<Self>) {
        match msg {
            ws::Message::Text(txt) => {
                if txt == self.payload.as_ref().as_str() {
                    self.perf_counters.register_request();
                    self.perf_counters
                        .register_latency(time::precise_time_ns() - self.ts);
                    if !self.send_text() {
                        ctx.stop();
                    }
                } else {
                    println!("not eaqual");
                }
            }
            _ => (),
        }
    }
}

pub struct PerfCounters {
    req: AtomicUsize,
    conn: AtomicUsize,
    lat: AtomicUsize,
    lat_max: AtomicUsize,
}

impl PerfCounters {
    pub fn new() -> PerfCounters {
        PerfCounters {
            req: AtomicUsize::new(0),
            conn: AtomicUsize::new(0),
            lat: AtomicUsize::new(0),
            lat_max: AtomicUsize::new(0),
        }
    }

    pub fn pull_request_count(&self) -> usize {
        self.req.swap(0, Ordering::SeqCst)
    }

    pub fn pull_connections_count(&self) -> usize {
        self.conn.swap(0, Ordering::SeqCst)
    }

    pub fn pull_latency_ns(&self) -> u64 {
        self.lat.swap(0, Ordering::SeqCst) as u64
    }

    pub fn pull_latency_max_ns(&self) -> u64 {
        self.lat_max.swap(0, Ordering::SeqCst) as u64
    }

    pub fn register_request(&self) {
        self.req.fetch_add(1, Ordering::SeqCst);
    }

    pub fn register_connection(&self) {
        self.conn.fetch_add(1, Ordering::SeqCst);
    }

    pub fn register_latency(&self, nanos: u64) {
        let nanos = nanos as usize;
        self.lat.fetch_add(nanos, Ordering::SeqCst);
        loop {
            let current = self.lat_max.load(Ordering::SeqCst);
            if current >= nanos
                || self.lat_max.compare_and_swap(current, nanos, Ordering::SeqCst)
                    == current
            {
                break;
            }
        }
    }
}
