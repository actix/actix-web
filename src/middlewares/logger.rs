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
/// %a %t "%r" %s %b "%{Referrer}i" "%{User-Agent}i %T"
/// ```
/// ```rust,ignore
///
/// let app = Application::default("/")
///     .middleware(Logger::default())
///     .middleware(Logger::new("%a %{User-Agent}i"))
///     .finish()
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
    /// Create default `Logger` middleware
    fn default() -> Logger {
        Logger { format: Format::default() }
    }
}

struct StartTime(time::Tm);

impl Logger {

    fn log(&self, req: &mut HttpRequest, resp: &HttpResponse) {
        let entry_time = req.extensions().get::<StartTime>().unwrap().0;

        let response_time = time::now() - entry_time;
        let response_time_ms = (response_time.num_seconds() * 1000) as f64 +
            (response_time.num_nanoseconds().unwrap_or(0) as f64) / 1000000.0;
        {
            let render = |fmt: &mut Formatter, text: &FormatText| {
                match *text {
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
                    FormatText::Time =>
                        fmt.write_fmt(format_args!("{:.6}", response_time_ms/1000.0)),
                    FormatText::TimeMillis =>
                        fmt.write_fmt(format_args!("{:.6}", response_time_ms)),
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
            };

            info!("{}", self.format.display_with(&render));
        }
    }
}

impl Middleware for Logger {

    fn start(&self, req: &mut HttpRequest) -> Started {
        req.extensions().insert(StartTime(time::now()));
        Started::Done
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
    ///
    /// ```ignore
    /// %a %t "%r" %s %b "%{Referrer}i" "%{User-Agent}i %T"
    /// ```
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

pub(crate) trait ContextDisplay<'a> {
    type Item;
    type Display: fmt::Display;
    fn display_with(&'a self,
                    render: &'a Fn(&mut Formatter, &Self::Item) -> Result<(), fmt::Error>)
                    -> Self::Display;
}

impl<'a> ContextDisplay<'a> for Format {
    type Item = FormatText;
    type Display = FormatDisplay<'a>;
    fn display_with(&'a self,
                    render: &'a Fn(&mut Formatter, &FormatText) -> Result<(), fmt::Error>)
                    -> FormatDisplay<'a> {
        FormatDisplay {
            format: self,
            render: render,
        }
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

pub(crate) struct FormatDisplay<'a> {
    format: &'a Format,
    render: &'a Fn(&mut Formatter, &FormatText) -> Result<(), fmt::Error>,
}

impl<'a> fmt::Display for FormatDisplay<'a> {
    fn fmt(&self, fmt: &mut Formatter) -> Result<(), fmt::Error> {
        let Format(ref format) = *self.format;
        for unit in format {
            (self.render)(fmt, unit)?;
        }
        Ok(())
    }
}
