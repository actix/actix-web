//! Request logging middleware
use std::collections::HashSet;
use std::env;
use std::fmt::{self, Display, Formatter};

use libc;
use regex::Regex;
use time;

use error::Result;
use httpmessage::HttpMessage;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use middleware::{Finished, Middleware, Started};

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
///  %a %t "%r" %s %b "%{Referer}i" "%{User-Agent}i" %T
/// ```
/// ```rust
/// # extern crate actix_web;
/// extern crate env_logger;
/// use actix_web::middleware::Logger;
/// use actix_web::App;
///
/// fn main() {
///     std::env::set_var("RUST_LOG", "actix_web=info");
///     env_logger::init();
///
///     let app = App::new()
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
/// `%T` Time taken to serve the request, in seconds with floating fraction in
/// .06f format
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
    exclude: HashSet<String>,
}

impl Logger {
    /// Create `Logger` middleware with the specified `format`.
    pub fn new(format: &str) -> Logger {
        Logger {
            format: Format::new(format),
            exclude: HashSet::new(),
        }
    }

    /// Ignore and do not log access info for specified path.
    pub fn exclude<T: Into<String>>(mut self, path: T) -> Self {
        self.exclude.insert(path.into());
        self
    }
}

impl Default for Logger {
    /// Create `Logger` middleware with format:
    ///
    /// ```ignore
    /// %a %t "%r" %s %b "%{Referer}i" "%{User-Agent}i" %T
    /// ```
    fn default() -> Logger {
        Logger {
            format: Format::default(),
            exclude: HashSet::new(),
        }
    }
}

struct StartTime(time::Tm);

impl Logger {
    fn log<S>(&self, req: &mut HttpRequest<S>, resp: &HttpResponse) {
        if let Some(entry_time) = req.extensions().get::<StartTime>() {
            let render = |fmt: &mut Formatter| {
                for unit in &self.format.0 {
                    unit.render(fmt, req, resp, entry_time.0)?;
                }
                Ok(())
            };
            info!("{}", FormatDisplay(&render));
        }
    }
}

impl<S> Middleware<S> for Logger {
    fn start(&self, req: &mut HttpRequest<S>) -> Result<Started> {
        if !self.exclude.contains(req.path()) {
            req.extensions_mut().insert(StartTime(time::now()));
        }
        Ok(Started::Done)
    }

    fn finish(&self, req: &mut HttpRequest<S>, resp: &HttpResponse) -> Finished {
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
        Format::new(r#"%a %t "%r" %s %b "%{Referer}i" "%{User-Agent}i" %T"#)
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
                results.push(match cap.get(3).unwrap().as_str() {
                    "i" => FormatText::RequestHeader(key.as_str().to_owned()),
                    "o" => FormatText::ResponseHeader(key.as_str().to_owned()),
                    "e" => FormatText::EnvironHeader(key.as_str().to_owned()),
                    _ => unreachable!(),
                })
            } else {
                let m = cap.get(1).unwrap();
                results.push(match m.as_str() {
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
    fn render<S>(
        &self, fmt: &mut Formatter, req: &HttpRequest<S>, resp: &HttpResponse,
        entry_time: time::Tm,
    ) -> Result<(), fmt::Error> {
        match *self {
            FormatText::Str(ref string) => fmt.write_str(string),
            FormatText::Percent => "%".fmt(fmt),
            FormatText::RequestLine => {
                if req.query_string().is_empty() {
                    fmt.write_fmt(format_args!(
                        "{} {} {:?}",
                        req.method(),
                        req.path(),
                        req.version()
                    ))
                } else {
                    fmt.write_fmt(format_args!(
                        "{} {}?{} {:?}",
                        req.method(),
                        req.path(),
                        req.query_string(),
                        req.version()
                    ))
                }
            }
            FormatText::ResponseStatus => resp.status().as_u16().fmt(fmt),
            FormatText::ResponseSize => resp.response_size().fmt(fmt),
            FormatText::Pid => unsafe { libc::getpid().fmt(fmt) },
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
            FormatText::RemoteAddr => {
                if let Some(remote) = req.connection_info().remote() {
                    return remote.fmt(fmt);
                } else {
                    "-".fmt(fmt)
                }
            }
            FormatText::RequestTime => entry_time
                .strftime("[%d/%b/%Y:%H:%M:%S %z]")
                .unwrap()
                .fmt(fmt),
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
                fmt.write_fmt(format_args!("{}", s))
            }
            FormatText::ResponseHeader(ref name) => {
                let s = if let Some(val) = resp.headers().get(name) {
                    if let Ok(s) = val.to_str() {
                        s
                    } else {
                        "-"
                    }
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

pub(crate) struct FormatDisplay<'a>(&'a Fn(&mut Formatter) -> Result<(), fmt::Error>);

impl<'a> fmt::Display for FormatDisplay<'a> {
    fn fmt(&self, fmt: &mut Formatter) -> Result<(), fmt::Error> {
        (self.0)(fmt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::header::{self, HeaderMap};
    use http::{Method, StatusCode, Uri, Version};
    use std::str::FromStr;
    use time;

    #[test]
    fn test_logger() {
        let logger = Logger::new("%% %{User-Agent}i %{X-Test}o %{HOME}e %D test");

        let mut headers = HeaderMap::new();
        headers.insert(
            header::USER_AGENT,
            header::HeaderValue::from_static("ACTIX-WEB"),
        );
        let mut req = HttpRequest::new(
            Method::GET,
            Uri::from_str("/").unwrap(),
            Version::HTTP_11,
            headers,
            None,
        );
        let resp = HttpResponse::build(StatusCode::OK)
            .header("X-Test", "ttt")
            .force_close()
            .finish();

        match logger.start(&mut req) {
            Ok(Started::Done) => (),
            _ => panic!(),
        };
        match logger.finish(&mut req, &resp) {
            Finished::Done => (),
            _ => panic!(),
        }
        let entry_time = time::now();
        let render = |fmt: &mut Formatter| {
            for unit in &logger.format.0 {
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
        headers.insert(
            header::USER_AGENT,
            header::HeaderValue::from_static("ACTIX-WEB"),
        );
        let req = HttpRequest::new(
            Method::GET,
            Uri::from_str("/").unwrap(),
            Version::HTTP_11,
            headers,
            None,
        );
        let resp = HttpResponse::build(StatusCode::OK).force_close().finish();
        let entry_time = time::now();

        let render = |fmt: &mut Formatter| {
            for unit in &format.0 {
                unit.render(fmt, &req, &resp, entry_time)?;
            }
            Ok(())
        };
        let s = format!("{}", FormatDisplay(&render));
        assert!(s.contains("GET / HTTP/1.1"));
        assert!(s.contains("200 0"));
        assert!(s.contains("ACTIX-WEB"));

        let req = HttpRequest::new(
            Method::GET,
            Uri::from_str("/?test").unwrap(),
            Version::HTTP_11,
            HeaderMap::new(),
            None,
        );
        let resp = HttpResponse::build(StatusCode::OK).force_close().finish();
        let entry_time = time::now();

        let render = |fmt: &mut Formatter| {
            for unit in &format.0 {
                unit.render(fmt, &req, &resp, entry_time)?;
            }
            Ok(())
        };
        let s = format!("{}", FormatDisplay(&render));
        assert!(s.contains("GET /?test HTTP/1.1"));
    }
}
