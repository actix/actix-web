//! Request logging middleware
use std::env;
use std::fmt;
use std::fmt::{Display, Formatter};

use libc;
use time;
use regex::Regex;

use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use middlewares::{Middleware, Started, Finished};

/// `Middleware` for logging request and response info to the terminal.
///
/// ## Usage
///
/// Create `Logger` middlewares with the specified `format`.
/// Default `Logger` could be created with `default` method, it uses the default format:
///
/// ```ignore
///  %a %t "%r" %s %b "%{Referrer}i" "%{User-Agent}i" %T
/// ```
/// ```rust
/// extern crate actix_web;
/// use actix_web::Application;
/// use actix_web::middlewares::Logger;
///
/// fn main() {
///     let app = Application::default("/")
///         .middleware(Logger::default())
///         .middleware(Logger::new("%a %{User-Agent}i"))
///         .finish();
/// }
/// ```
///
/// ## Format
///
/// `%%`  The percent sign
///
/// `%a`  Remote IP-address (IP-address of proxy if using reverse proxy)
///
/// `%t`  Time when the request was started to process
///
/// `%P`  The process ID of the child that serviced the request
///
/// `%r`  First line of request
///
/// `%s`  Response status code
///
/// `%b`  Size of response in bytes, including HTTP headers
///
/// `%T` Time taken to serve the request, in seconds with floating fraction in .06f format
///
/// `%D`  Time taken to serve the request, in milliseconds
///
/// `%{FOO}i`  request.headers['FOO']
///
/// `%{FOO}o`  response.headers['FOO']
///
/// `%{FOO}e`  os.environ['FOO']
///
pub struct Logger {
    format: Format,
}

impl Logger {
    /// Create `Logger` middleware with the specified `format`.
    pub fn new(format: &str) -> Logger {
        Logger { format: Format::new(format) }
    }
}

impl Default for Logger {
    /// Create `Logger` middleware with format:
    ///
    /// ```ignore
    /// %a %t "%r" %s %b "%{Referrer}i" "%{User-Agent}i" %T
    /// ```
    fn default() -> Logger {
        Logger { format: Format::default() }
    }
}

struct StartTime(time::Tm);

impl Logger {

    fn log(&self, req: &mut HttpRequest, resp: &HttpResponse) {
        let entry_time = req.extensions().get::<StartTime>().unwrap().0;

        let render = |fmt: &mut Formatter| {
            for unit in &self.format.0 {
                unit.render(fmt, req, resp, entry_time)?;
            }
            Ok(())
        };
        info!("{}", FormatDisplay(&render));
    }
}

impl Middleware for Logger {

    fn start(&self, mut req: HttpRequest) -> Started {
        req.extensions().insert(StartTime(time::now()));
        Started::Done(req)
    }

    fn finish(&self, req: &mut HttpRequest, resp: &HttpResponse) -> Finished {
        self.log(req, resp);
        Finished::Done
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
        Format::new(r#"%a %t "%r" %s %b "%{Referrer}i" "%{User-Agent}i" %T"#)
    }
}

impl Format {
    /// Create a `Format` from a format string.
    ///
    /// Returns `None` if the format string syntax is incorrect.
    pub fn new(s: &str) -> Format {
        trace!("Access log format: {}", s);
        let fmt = Regex::new(r"%(\{([A-Za-z0-9\-_]+)\}([ioe])|[atPrsbTD]?)").unwrap();

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
                results.push(
                    match cap.get(3).unwrap().as_str() {
                        "i" => FormatText::RequestHeader(key.as_str().to_owned()),
                        "o" => FormatText::ResponseHeader(key.as_str().to_owned()),
                        "e" => FormatText::EnvironHeader(key.as_str().to_owned()),
                        _ => unreachable!(),
                    })
            } else {
                let m = cap.get(1).unwrap();
                results.push(
                    match m.as_str() {
                        "%" => FormatText::Percent,
                        "a" => FormatText::RemoteAddr,
                        "t" => FormatText::RequestTime,
                        "P" => FormatText::Pid,
                        "r" => FormatText::RequestLine,
                        "s" => FormatText::ResponseStatus,
                        "b" => FormatText::ResponseSize,
                        "T" => FormatText::Time,
                        "D" => FormatText::TimeMillis,
                        _ => FormatText::Str(m.as_str().to_owned()),
                    }
                );
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
    Pid,
    Percent,
    RequestLine,
    RequestTime,
    ResponseStatus,
    ResponseSize,
    Time,
    TimeMillis,
    RemoteAddr,
    RequestHeader(String),
    ResponseHeader(String),
    EnvironHeader(String),
}

impl FormatText {

    fn render(&self, fmt: &mut Formatter,
              req: &HttpRequest,
              resp: &HttpResponse,
              entry_time: time::Tm) -> Result<(), fmt::Error>
    {
        match *self {
            FormatText::Str(ref string) => fmt.write_str(string),
            FormatText::Percent => "%".fmt(fmt),
            FormatText::RequestLine => {
                if req.query_string().is_empty() {
                    fmt.write_fmt(format_args!(
                        "{} {} {:?}",
                        req.method(), req.path(), req.version()))
                } else {
                    fmt.write_fmt(format_args!(
                        "{} {}?{} {:?}",
                        req.method(), req.path(), req.query_string(), req.version()))
                }
            },
            FormatText::ResponseStatus => resp.status().as_u16().fmt(fmt),
            FormatText::ResponseSize => resp.response_size().fmt(fmt),
            FormatText::Pid => unsafe{libc::getpid().fmt(fmt)},
            FormatText::Time => {
                let response_time = time::now() - entry_time;
                let response_time = (response_time.num_seconds() * 1000) as f64 +
                    (response_time.num_nanoseconds().unwrap_or(0) as f64) / 1000000000.0;

                fmt.write_fmt(format_args!("{:.6}", response_time))
            },
            FormatText::TimeMillis => {
                let response_time = time::now() - entry_time;
                let response_time_ms = (response_time.num_seconds() * 1000) as f64 +
                    (response_time.num_nanoseconds().unwrap_or(0) as f64) / 1000000.0;

                fmt.write_fmt(format_args!("{:.6}", response_time_ms))
            },
            FormatText::RemoteAddr => {
                if let Some(addr) = req.remote() {
                    addr.fmt(fmt)
                } else {
                    "-".fmt(fmt)
                }
            }
            FormatText::RequestTime => {
                entry_time.strftime("[%d/%b/%Y:%H:%M:%S %z]")
                    .unwrap()
                    .fmt(fmt)
            }
            FormatText::RequestHeader(ref name) => {
                let s = if let Some(val) = req.headers().get(name) {
                    if let Ok(s) = val.to_str() { s } else { "-" }
                } else {
                    "-"
                };
                fmt.write_fmt(format_args!("{}", s))
            }
            FormatText::ResponseHeader(ref name) => {
                let s = if let Some(val) = resp.headers().get(name) {
                    if let Ok(s) = val.to_str() { s } else { "-" }
                } else {
                    "-"
                };
                fmt.write_fmt(format_args!("{}", s))
            }
            FormatText::EnvironHeader(ref name) => {
                if let Ok(val) = env::var(name) {
                    fmt.write_fmt(format_args!("{}", val))
                } else {
                    "-".fmt(fmt)
                }
            }
        }
    }
}

pub(crate) struct FormatDisplay<'a>(
    &'a Fn(&mut Formatter) -> Result<(), fmt::Error>);

impl<'a> fmt::Display for FormatDisplay<'a> {
    fn fmt(&self, fmt: &mut Formatter) -> Result<(), fmt::Error> {
        (self.0)(fmt)
    }
}

#[cfg(test)]
mod tests {
    use Body;
    use super::*;
    use time;
    use http::{Method, Version, StatusCode};
    use http::header::{self, HeaderMap};

    #[test]
    fn test_logger() {
        let logger = Logger::new("%% %{User-Agent}i %{X-Test}o %{HOME}e %D test");

        let mut headers = HeaderMap::new();
        headers.insert(header::USER_AGENT, header::HeaderValue::from_static("ACTIX-WEB"));
        let req = HttpRequest::new(
            Method::GET, "/".to_owned(), Version::HTTP_11, headers, String::new());
        let resp = HttpResponse::builder(StatusCode::OK)
            .header("X-Test", "ttt")
            .force_close().body(Body::Empty).unwrap();

        let mut req = match logger.start(req) {
            Started::Done(req) => req,
            _ => panic!(),
        };
        match logger.finish(&mut req, &resp) {
            Finished::Done => (),
            _ => panic!(),
        }
        let entry_time = time::now();
        let render = |fmt: &mut Formatter| {
            for unit in logger.format.0.iter() {
                unit.render(fmt, &req, &resp, entry_time)?;
            }
            Ok(())
        };
        let s = format!("{}", FormatDisplay(&render));
        assert!(s.contains("ACTIX-WEB ttt"));
    }

    #[test]
    fn test_default_format() {
        let format = Format::default();

        let mut headers = HeaderMap::new();
        headers.insert(header::USER_AGENT, header::HeaderValue::from_static("ACTIX-WEB"));
        let req = HttpRequest::new(
            Method::GET, "/".to_owned(), Version::HTTP_11, headers, String::new());
        let resp = HttpResponse::builder(StatusCode::OK)
            .force_close().body(Body::Empty).unwrap();
        let entry_time = time::now();

        let render = |fmt: &mut Formatter| {
            for unit in format.0.iter() {
                unit.render(fmt, &req, &resp, entry_time)?;
            }
            Ok(())
        };
        let s = format!("{}", FormatDisplay(&render));
        assert!(s.contains("GET / HTTP/1.1"));
        assert!(s.contains("200 0"));
        assert!(s.contains("ACTIX-WEB"));

        let req = HttpRequest::new(
            Method::GET, "/".to_owned(), Version::HTTP_11, HeaderMap::new(), "test".to_owned());
        let resp = HttpResponse::builder(StatusCode::OK)
            .force_close().body(Body::Empty).unwrap();
        let entry_time = time::now();

        let render = |fmt: &mut Formatter| {
            for unit in format.0.iter() {
                unit.render(fmt, &req, &resp, entry_time)?;
            }
            Ok(())
        };
        let s = format!("{}", FormatDisplay(&render));
        assert!(s.contains("GET /?test HTTP/1.1"));
    }
}
