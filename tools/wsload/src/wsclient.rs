//! Simple websocket client.

#![allow(unused_variables)]
extern crate actix;
extern crate actix_web;
extern crate env_logger;
extern crate futures;
extern crate tokio_core;
extern crate url;
extern crate clap;
extern crate rand;
extern crate time;
extern crate num_cpus;

use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use futures::Future;
use rand::{thread_rng, Rng};

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
                -c, --concurrency=[NUMBER] 'number of websockt connections to open and use concurrently for sending'
                -t, --threads=[NUMBER] 'number of threads to use'",
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
        Some(s) => parse_u64_default(Some(s), 0) as usize * 1024,
        None => 1024,
    };
    let warmup_seconds = parse_u64_default(matches.value_of("warm-up"), 2) as u64;
    let sample_rate = parse_u64_default(matches.value_of("sample-rate"), 1) as usize;

    let perf_counters = Arc::new(PerfCounters::new());
    let payload = Arc::new(thread_rng()
                           .gen_ascii_chars()
                           .take(payload_size)
                           .collect::<String>());

    let sys = actix::System::new("ws-client");

    let mut report = true;
    for t in 0..threads {
        let pl = payload.clone();
        let ws = ws_url.clone();
        let perf = perf_counters.clone();
        let addr = Arbiter::new(format!("test {}", t));

        addr.do_send(actix::msgs::Execute::new(move || -> Result<(), ()> {
            let mut reps = report;
            for _ in 0..concurrency {
                let pl2 = pl.clone();
                let perf2 = perf.clone();

                Arbiter::handle().spawn(
                    ws::Client::new(&ws).connect()
                        .map_err(|e| {
                            println!("Error: {}", e);
                            Arbiter::system().do_send(actix::msgs::SystemExit(0));
                            ()
                        })
                        .map(move |(reader, writer)| {
                            let addr: Addr<Syn, _> = ChatClient::create(move |ctx| {
                                ChatClient::add_stream(reader, ctx);
                                ChatClient{conn: writer,
                                           payload: pl2,
                                           report: reps,
                                           bin: bin,
                                           ts: time::precise_time_ns(),
                                           perf_counters: perf2,
                                           sample_rate_secs: sample_rate,
                                }
                            });
                        })
                );
                reps = false;
            }
            Ok(())
        }));
        report = false;
    }

    let _ = sys.run();
}

fn parse_u64_default(input: Option<&str>, default: u64) -> u64 {
    input.map(|v| v.parse().expect(&format!("not a valid number: {}", v)))
        .unwrap_or(default)
}

struct ChatClient{
    conn: ws::ClientWriter,
    payload: Arc<String>,
    ts: u64,
    bin: bool,
    report: bool,
    perf_counters: Arc<PerfCounters>,
    sample_rate_secs: usize,
}

impl Actor for ChatClient {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Context<Self>) {
        self.send_text();
        if self.report {
            self.sample_rate(ctx);
        }
    }

    fn stopping(&mut self, _: &mut Context<Self>) -> Running {
        Arbiter::system().do_send(actix::msgs::SystemExit(0));
        Running::Stop
    }
}

impl ChatClient {
    fn sample_rate(&self, ctx: &mut Context<Self>) {
        ctx.run_later(Duration::new(self.sample_rate_secs as u64, 0), |act, ctx| {
            let req_count = act.perf_counters.pull_request_count();
            if req_count != 0 {
                let latency = act.perf_counters.pull_latency_ns();
                let latency_max = act.perf_counters.pull_latency_max_ns();
                println!(
                    "rate: {}, throughput: {:?} kb, latency: {}, latency max: {}",
                    req_count / act.sample_rate_secs,
                    (((req_count * act.payload.len()) as f64) / 1024.0) /
                        act.sample_rate_secs as f64,
                    time::Duration::nanoseconds((latency / req_count as u64) as i64),
                    time::Duration::nanoseconds(latency_max as i64)
                );
            }

            act.sample_rate(ctx);
        });
    }

    fn send_text(&mut self) {
        self.ts = time::precise_time_ns();
        if self.bin {
            self.conn.binary(&self.payload);
        } else {
            self.conn.text(&self.payload);
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
                    self.perf_counters.register_latency(time::precise_time_ns() - self.ts);
                    self.send_text();
                } else {
                    println!("not eaqual");
                }
            },
            _ => ()
        }
    }
}


pub struct PerfCounters {
    req: AtomicUsize,
    lat: AtomicUsize,
    lat_max: AtomicUsize
}

impl PerfCounters {
    pub fn new() -> PerfCounters {
        PerfCounters {
            req: AtomicUsize::new(0),
            lat: AtomicUsize::new(0),
            lat_max: AtomicUsize::new(0),
        }
    }

    pub fn pull_request_count(&self) -> usize {
        self.req.swap(0, Ordering::SeqCst)
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

    pub fn register_latency(&self, nanos: u64) {
        let nanos = nanos as usize;
        self.lat.fetch_add(nanos, Ordering::SeqCst);
        loop {
            let current = self.lat_max.load(Ordering::SeqCst);
            if current >= nanos || self.lat_max.compare_and_swap(current, nanos, Ordering::SeqCst) == current {
                break;
            }
        }
    }
}
