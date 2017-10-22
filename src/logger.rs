//! Request logging middleware
use std::fmt;
use std::str::Chars;
use std::iter::Peekable;
use std::fmt::{Display, Formatter};

use time;

use application::Middleware;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

/// `Middleware` for logging request and response info to the terminal.
pub struct Logger {
    format: Format,
}

impl Logger {
    /// Create `Logger` middlewares with the specified `format`.
    /// If a `None` is passed in, uses the default format:
    ///
    /// ```ignore
    /// {method} {uri} -> {status} ({response-time} ms)
    /// ```
    ///
    /// ```rust,ignore
    /// let app = Application::default("/")
    ///     .middleware(Logger::new(None))
    ///     .finish()
    /// ```
    pub fn new(format: Option<Format>) -> Logger {
        let format = format.unwrap_or_default();
        Logger { format: format.clone() }
    }
}

struct StartTime(time::Tm);

impl Logger {
    fn initialise(&self, req: &mut HttpRequest) {
        req.extensions().insert(StartTime(time::now()));
    }

    fn log(&self, req: &mut HttpRequest, resp: &HttpResponse) {
        let entry_time = req.extensions().get::<StartTime>().unwrap().0;

        let response_time = time::now() - entry_time;
        let response_time_ms = (response_time.num_seconds() * 1000) as f64 + (response_time.num_nanoseconds().unwrap_or(0) as f64) / 1000000.0;

        {
            let render = |fmt: &mut Formatter, text: &FormatText| {
                match *text {
                    FormatText::Str(ref string) => fmt.write_str(string),
                    FormatText::Method => req.method().fmt(fmt),
                    FormatText::URI => req.uri().fmt(fmt),
                    FormatText::Status => resp.status().fmt(fmt),
                    FormatText::ResponseTime =>
                        fmt.write_fmt(format_args!("{} ms", response_time_ms)),
                    FormatText::RemoteAddr => Ok(()), //req.remote_addr.fmt(fmt),
                    FormatText::RequestTime => {
                        entry_time.strftime("%Y-%m-%dT%H:%M:%S.%fZ%z")
                            .unwrap()
                            .fmt(fmt)
                    }
                }
            };

            info!("{}", self.format.display_with(&render));
            //println!("{}", self.format.display_with(&render));
        }
    }
}

impl Middleware for Logger {
    fn start(&self, req: &mut HttpRequest) -> Result<(), HttpResponse> {
        self.initialise(req);
        Ok(())
    }

    fn finish(&self, req: &mut HttpRequest, resp: &HttpResponse) {
        self.log(req, resp);
    }
}


use self::FormatText::{Method, URI, Status, ResponseTime, RemoteAddr, RequestTime};

/// A formatting style for the `Logger`, consisting of multiple
/// `FormatText`s concatenated into one line.
#[derive(Clone)]
#[doc(hidden)]
pub struct Format(Vec<FormatText>);

impl Default for Format {
    /// Return the default formatting style for the `Logger`:
    ///
    /// ```ignore
    /// {method} {uri} -> {status} ({response-time})
    /// // This will be written as: {method} {uri} -> {status} ({response-time})
    /// ```
    fn default() -> Format {
        Format::new("{method} {uri} {status} ({response-time})").unwrap()
    }
}

impl Format {
    /// Create a `Format` from a format string, which can contain the fields
    /// `{method}`, `{uri}`, `{status}`, `{response-time}`, `{ip-addr}` and
    /// `{request-time}`.
    ///
    /// Returns `None` if the format string syntax is incorrect.
    pub fn new(s: &str) -> Option<Format> {

        let parser = FormatParser::new(s.chars().peekable());

        let mut results = Vec::new();

        for unit in parser {
            match unit {
                Some(unit) => results.push(unit),
                None => return None
            }
        }

        Some(Format(results))
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

struct FormatParser<'a> {
    // The characters of the format string.
    chars: Peekable<Chars<'a>>,

    // A reusable buffer for parsing style attributes.
    object_buffer: String,

    finished: bool
}

impl<'a> FormatParser<'a> {
    fn new(chars: Peekable<Chars>) -> FormatParser {
        FormatParser {
            chars: chars,

            // No attributes are longer than 14 characters, so we can avoid reallocating.
            object_buffer: String::with_capacity(14),

            finished: false
        }
    }
}

// Some(None) means there was a parse error and this FormatParser should be abandoned.
impl<'a> Iterator for FormatParser<'a> {
    type Item = Option<FormatText>;

    fn next(&mut self) -> Option<Option<FormatText>> {
        // If the parser has been cancelled or errored for some reason.
        if self.finished { return None }

        // Try to parse a new FormatText.
        match self.chars.next() {
            // Parse a recognized object.
            //
            // The allowed forms are:
            //   - {method}
            //   - {uri}
            //   - {status}
            //   - {response-time}
            //   - {ip-addr}
            //   - {request-time}
            Some('{') => {
                self.object_buffer.clear();

                let mut chr = self.chars.next();
                while chr != None {
                    match chr.unwrap() {
                        // Finished parsing, parse buffer.
                        '}' => break,
                        c => self.object_buffer.push(c.clone())
                    }

                    chr = self.chars.next();
                }

                let text = match self.object_buffer.as_ref() {
                    "method" => Method,
                    "uri" => URI,
                    "status" => Status,
                    "response-time" => ResponseTime,
                    "request-time" => RequestTime,
                    "ip-addr" => RemoteAddr,
                    _ => {
                        // Error, so mark as finished.
                        self.finished = true;
                        return Some(None);
                    }
                };

                Some(Some(text))
            }

            // Parse a regular string part of the format string.
            Some(c) => {
                let mut buffer = String::new();
                buffer.push(c);

                loop {
                    match self.chars.peek() {
                        // Done parsing.
                        Some(&'{') | None => return Some(Some(FormatText::Str(buffer))),

                        Some(_) => {
                            buffer.push(self.chars.next().unwrap())
                        }
                    }
                }
            },

            // Reached end of the format string.
            None => None
        }
    }
}

/// A string of text to be logged. This is either one of the data
/// fields supported by the `Logger`, or a custom `String`.
#[derive(Clone)]
#[doc(hidden)]
pub enum FormatText {
    Str(String),
    Method,
    URI,
    Status,
    ResponseTime,
    RemoteAddr,
    RequestTime
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
