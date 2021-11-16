/// Body size hint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BodySize {
    /// Absence of body can be assumed from method or status code.
    ///
    /// Will skip writing Content-Length header.
    None,

    /// Known size body.
    ///
    /// Will write `Content-Length: N` header.
    Sized(u64),

    /// Unknown size body.
    ///
    /// Will not write Content-Length header. Can be used with chunked Transfer-Encoding.
    Stream,
}

impl BodySize {
    /// Returns true if size hint indicates no or empty body.
    ///
    /// Streams will return false because it cannot be known without reading the stream.
    ///
    /// ```
    /// # use actix_http::body::BodySize;
    /// assert!(BodySize::None.is_eof());
    /// assert!(BodySize::Empty.is_eof());
    /// assert!(BodySize::Sized(0).is_eof());
    ///
    /// assert!(!BodySize::Sized(64).is_eof());
    /// assert!(!BodySize::Stream.is_eof());
    /// ```
    pub fn is_eof(&self) -> bool {
        matches!(self, BodySize::None | BodySize::Sized(0))
    }
}
