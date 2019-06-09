//! Request logging middleware
use std::collections::HashSet;
use std::env;
use std::fmt::{self, Display, Formatter};
use std::marker::PhantomData;
use std::rc::Rc;

use actix_service::{Service, Transform};
use bytes::Bytes;
use futures::future::{ok, FutureResult};
use futures::{Async, Future, Poll};
use regex::Regex;
use time;

use crate::dev::{BodySize, MessageBody, ResponseBody};
use crate::error::{Error, Result};
use crate::http::{HeaderName, HttpTryFrom};
use crate::service::{ServiceRequest, ServiceResponse};
use crate::HttpResponse;

/// `Middleware` for logging request and response info to the terminal.
///
/// `Logger` middleware uses standard log crate to log information. You should
/// enable logger for `actix_web` package to see access log.
/// ([`env_logger`](https://docs.rs/env_logger/*/env_logger/) or similar)
///
/// ## Usage
///
/// Create `Logger` middleware with the specified `format`.
/// Default `Logger` could be created with `default` method, it uses the
/// default format:
///
/// ```ignore
///  %a "%r" %s %b "%{Referer}i" "%{User-Agent}i" %T
/// ```
/// ```rust
/// use actix_web::middleware::Logger;
/// use actix_web::App;
///
/// fn main() {
///     std::env::set_var("RUST_LOG", "actix_web=info");
///     env_logger::init();
///
///     let app = App::new()
///         .wrap(Logger::default())
///         .wrap(Logger::new("%a %{User-Agent}i"));
/// }
/// ```
///
/// ## Format
///
/// `%%`  The percent sign
///
/// `%a`  Remote IP-address (IP-address of proxy if using reverse proxy)
///
/// `%t`  Time when the request was started to process (in rfc3339 format)
///
/// `%r`  First line of request
///
/// `%s`  Response status code
///
/// `%b`  Size of response in bytes, including HTTP headers
///
/// `%T` Time taken to serve the request, in seconds with floating fraction in
/// .06f format
///
/// `%D`  Time taken to serve the request, in milliseconds
///
/// `%U`  Request URL
///
/// `%{FOO}i`  request.headers['FOO']
///
/// `%{FOO}o`  response.headers['FOO']
///
/// `%{FOO}e`  os.environ['FOO']
///
pub struct Logger(Rc<Inner>);

struct Inner {
    format: Format,
    exclude: HashSet<String>,
}

impl Logger {
    /// Create `Logger` middleware with the specified `format`.
    pub fn new(format: &str) -> Logger {
        Logger(Rc::new(Inner {
            format: Format::new(format),
            exclude: HashSet::new(),
        }))
    }

    /// Ignore and do not log access info for specified path.
    pub fn exclude<T: Into<String>>(mut self, path: T) -> Self {
        Rc::get_mut(&mut self.0)
            .unwrap()
            .exclude
            .insert(path.into());
        self
    }
}

impl Default for Logger {
    /// Create `Logger` middleware with format:
    ///
    /// ```ignore
    /// %a "%r" %s %b "%{Referer}i" "%{User-Agent}i" %T
    /// ```
    fn default() -> Logger {
        Logger(Rc::new(Inner {
            format: Format::default(),
            exclude: HashSet::new(),
        }))
    }
}

impl<S, B> Transform<S> for Logger
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    B: MessageBody,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<StreamLog<B>>;
    type Error = Error;
    type InitError = ();
    type Transform = LoggerMiddleware<S>;
    type Future = FutureResult<Self::Transform, Self::InitError>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(LoggerMiddleware {
            service,
            inner: self.0.clone(),
        })
    }
}

/// Logger middleware
pub struct LoggerMiddleware<S> {
    inner: Rc<Inner>,
    service: S,
}

impl<S, B> Service for LoggerMiddleware<S>
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    B: MessageBody,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<StreamLog<B>>;
    type Error = Error;
    type Future = LoggerResponse<S, B>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready()
    }

    fn call(&mut self, req: ServiceRequest) -> Self::Future {
        if self.inner.exclude.contains(req.path()) {
            LoggerResponse {
                fut: self.service.call(req),
                format: None,
                time: time::now(),
                _t: PhantomData,
            }
        } else {
            let now = time::now();
            let mut format = self.inner.format.clone();

            for unit in &mut format.0 {
                unit.render_request(now, &req);
            }
            LoggerResponse {
                fut: self.service.call(req),
                format: Some(format),
                time: now,
                _t: PhantomData,
            }
        }
    }
}

#[doc(hidden)]
pub struct LoggerResponse<S, B>
where
    B: MessageBody,
    S: Service,
{
    fut: S::Future,
    time: time::Tm,
    format: Option<Format>,
    _t: PhantomData<(B,)>,
}

impl<S, B> Future for LoggerResponse<S, B>
where
    B: MessageBody,
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
{
    type Item = ServiceResponse<StreamLog<B>>;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let res = futures::try_ready!(self.fut.poll());

        if let Some(ref mut format) = self.format {
            for unit in &mut format.0 {
                unit.render_response(res.response());
            }
        }

        Ok(Async::Ready(res.map_body(move |_, body| {
            ResponseBody::Body(StreamLog {
                body,
                size: 0,
                time: self.time,
                format: self.format.take(),
            })
        })))
    }
}

pub struct StreamLog<B> {
    body: ResponseBody<B>,
    format: Option<Format>,
    size: usize,
    time: time::Tm,
}

impl<B> Drop for StreamLog<B> {
    fn drop(&mut self) {
        if let Some(ref format) = self.format {
            let render = |fmt: &mut Formatter| {
                for unit in &format.0 {
                    unit.render(fmt, self.size, self.time)?;
                }
                Ok(())
            };
            log::info!("{}", FormatDisplay(&render));
        }
    }
}

impl<B: MessageBody> MessageBody for StreamLog<B> {
    fn size(&self) -> BodySize {
        self.body.size()
    }

    fn poll_next(&mut self) -> Poll<Option<Bytes>, Error> {
        match self.body.poll_next()? {
            Async::Ready(Some(chunk)) => {
                self.size += chunk.len();
                Ok(Async::Ready(Some(chunk)))
            }
            val => Ok(val),
        }
    }
}

/// A formatting style for the `Logger`, consisting of multiple
/// `FormatText`s concatenated into one line.
#[derive(Clone)]
#[doc(hidden)]
struct Format(Vec<FormatText>);

impl Default for Format {
    /// Return the default formatting style for the `Logger`:
    fn default() -> Format {
        Format::new(r#"%a "%r" %s %b "%{Referer}i" "%{User-Agent}i" %T"#)
    }
}

impl Format {
    /// Create a `Format` from a format string.
    ///
    /// Returns `None` if the format string syntax is incorrect.
    pub fn new(s: &str) -> Format {
        log::trace!("Access log format: {}", s);
        let fmt = Regex::new(r"%(\{([A-Za-z0-9\-_]+)\}([ioe])|[atPrUsbTD]?)").unwrap();

        let mut idx = 0;
        let mut results = Vec::new();
        for cap in fmt.captures_iter(s) {
            let m = cap.get(0).unwrap();
            let pos = m.start();
            if idx != pos {
                results.push(FormatText::Str(s[idx..pos].to_owned()));
            }
            idx = m.end();

            if let Some(key) = cap.get(2) {
                results.push(match cap.get(3).unwrap().as_str() {
                    "i" => FormatText::RequestHeader(
                        HeaderName::try_from(key.as_str()).unwrap(),
                    ),
                    "o" => FormatText::ResponseHeader(
                        HeaderName::try_from(key.as_str()).unwrap(),
                    ),
                    "e" => FormatText::EnvironHeader(key.as_str().to_owned()),
                    _ => unreachable!(),
                })
            } else {
                let m = cap.get(1).unwrap();
                results.push(match m.as_str() {
                    "%" => FormatText::Percent,
                    "a" => FormatText::RemoteAddr,
                    "t" => FormatText::RequestTime,
                    "r" => FormatText::RequestLine,
                    "s" => FormatText::ResponseStatus,
                    "b" => FormatText::ResponseSize,
                    "U" => FormatText::UrlPath,
                    "T" => FormatText::Time,
                    "D" => FormatText::TimeMillis,
                    _ => FormatText::Str(m.as_str().to_owned()),
                });
            }
        }
        if idx != s.len() {
            results.push(FormatText::Str(s[idx..].to_owned()));
        }

        Format(results)
    }
}

/// A string of text to be logged. This is either one of the data
/// fields supported by the `Logger`, or a custom `String`.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub enum FormatText {
    Str(String),
    Percent,
    RequestLine,
    RequestTime,
    ResponseStatus,
    ResponseSize,
    Time,
    TimeMillis,
    RemoteAddr,
    UrlPath,
    RequestHeader(HeaderName),
    ResponseHeader(HeaderName),
    EnvironHeader(String),
}

impl FormatText {
    fn render(
        &self,
        fmt: &mut Formatter,
        size: usize,
        entry_time: time::Tm,
    ) -> Result<(), fmt::Error> {
        match *self {
            FormatText::Str(ref string) => fmt.write_str(string),
            FormatText::Percent => "%".fmt(fmt),
            FormatText::ResponseSize => size.fmt(fmt),
            FormatText::Time => {
                let rt = time::now() - entry_time;
                let rt = (rt.num_nanoseconds().unwrap_or(0) as f64) / 1_000_000_000.0;
                fmt.write_fmt(format_args!("{:.6}", rt))
            }
            FormatText::TimeMillis => {
                let rt = time::now() - entry_time;
                let rt = (rt.num_nanoseconds().unwrap_or(0) as f64) / 1_000_000.0;
                fmt.write_fmt(format_args!("{:.6}", rt))
            }
            FormatText::EnvironHeader(ref name) => {
                if let Ok(val) = env::var(name) {
                    fmt.write_fmt(format_args!("{}", val))
                } else {
                    "-".fmt(fmt)
                }
            }
            _ => Ok(()),
        }
    }

    fn render_response<B>(&mut self, res: &HttpResponse<B>) {
        match *self {
            FormatText::ResponseStatus => {
                *self = FormatText::Str(format!("{}", res.status().as_u16()))
            }
            FormatText::ResponseHeader(ref name) => {
                let s = if let Some(val) = res.headers().get(name) {
                    if let Ok(s) = val.to_str() {
                        s
                    } else {
                        "-"
                    }
                } else {
                    "-"
                };
                *self = FormatText::Str(s.to_string())
            }
            _ => (),
        }
    }

    fn render_request(&mut self, now: time::Tm, req: &ServiceRequest) {
        match *self {
            FormatText::RequestLine => {
                *self = if req.query_string().is_empty() {
                    FormatText::Str(format!(
                        "{} {} {:?}",
                        req.method(),
                        req.path(),
                        req.version()
                    ))
                } else {
                    FormatText::Str(format!(
                        "{} {}?{} {:?}",
                        req.method(),
                        req.path(),
                        req.query_string(),
                        req.version()
                    ))
                };
            }
            FormatText::UrlPath => *self = FormatText::Str(format!("{}", req.path())),
            FormatText::RequestTime => {
                *self = FormatText::Str(format!("{}", now.rfc3339()))
            }
            FormatText::RequestHeader(ref name) => {
                let s = if let Some(val) = req.headers().get(name) {
                    if let Ok(s) = val.to_str() {
                        s
                    } else {
                        "-"
                    }
                } else {
                    "-"
                };
                *self = FormatText::Str(s.to_string());
            }
            FormatText::RemoteAddr => {
                let s = if let Some(remote) = req.connection_info().remote() {
                    FormatText::Str(remote.to_string())
                } else {
                    FormatText::Str("-".to_string())
                };
                *self = s;
            }
            _ => (),
        }
    }
}

pub(crate) struct FormatDisplay<'a>(&'a Fn(&mut Formatter) -> Result<(), fmt::Error>);

impl<'a> fmt::Display for FormatDisplay<'a> {
    fn fmt(&self, fmt: &mut Formatter) -> Result<(), fmt::Error> {
        (self.0)(fmt)
    }
}

#[cfg(test)]
mod tests {
    use actix_service::{IntoService, Service, Transform};

    use super::*;
    use crate::http::{header, StatusCode};
    use crate::test::{block_on, TestRequest};

    #[test]
    fn test_logger() {
        let srv = |req: ServiceRequest| {
            req.into_response(
                HttpResponse::build(StatusCode::OK)
                    .header("X-Test", "ttt")
                    .finish(),
            )
        };
        let logger = Logger::new("%% %{User-Agent}i %{X-Test}o %{HOME}e %D test");

        let mut srv = block_on(logger.new_transform(srv.into_service())).unwrap();

        let req = TestRequest::with_header(
            header::USER_AGENT,
            header::HeaderValue::from_static("ACTIX-WEB"),
        )
        .to_srv_request();
        let _res = block_on(srv.call(req));
    }

    #[test]
    fn test_url_path() {
        let mut format = Format::new("%T %U");
        let req = TestRequest::with_header(
            header::USER_AGENT,
            header::HeaderValue::from_static("ACTIX-WEB"),
        )
        .uri("/test/route/yeah")
        .to_srv_request();

        let now = time::now();
        for unit in &mut format.0 {
            unit.render_request(now, &req);
        }

        let resp = HttpResponse::build(StatusCode::OK).force_close().finish();
        for unit in &mut format.0 {
            unit.render_response(&resp);
        }

        let render = |fmt: &mut Formatter| {
            for unit in &format.0 {
                unit.render(fmt, 1024, now)?;
            }
            Ok(())
        };
        let s = format!("{}", FormatDisplay(&render));
        println!("{}", s);
        assert!(s.contains("/test/route/yeah"));
    }

    #[test]
    fn test_default_format() {
        let mut format = Format::default();

        let req = TestRequest::with_header(
            header::USER_AGENT,
            header::HeaderValue::from_static("ACTIX-WEB"),
        )
        .to_srv_request();

        let now = time::now();
        for unit in &mut format.0 {
            unit.render_request(now, &req);
        }

        let resp = HttpResponse::build(StatusCode::OK).force_close().finish();
        for unit in &mut format.0 {
            unit.render_response(&resp);
        }

        let entry_time = time::now();
        let render = |fmt: &mut Formatter| {
            for unit in &format.0 {
                unit.render(fmt, 1024, entry_time)?;
            }
            Ok(())
        };
        let s = format!("{}", FormatDisplay(&render));
        assert!(s.contains("GET / HTTP/1.1"));
        assert!(s.contains("200 1024"));
        assert!(s.contains("ACTIX-WEB"));
    }

    #[test]
    fn test_request_time_format() {
        let mut format = Format::new("%t");
        let req = TestRequest::default().to_srv_request();

        let now = time::now();
        for unit in &mut format.0 {
            unit.render_request(now, &req);
        }

        let resp = HttpResponse::build(StatusCode::OK).force_close().finish();
        for unit in &mut format.0 {
            unit.render_response(&resp);
        }

        let render = |fmt: &mut Formatter| {
            for unit in &format.0 {
                unit.render(fmt, 1024, now)?;
            }
            Ok(())
        };
        let s = format!("{}", FormatDisplay(&render));
        assert!(s.contains(&format!("{}", now.rfc3339())));
    }
}
