/// Body size hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodySize {
    /// Implicitly empty body.
    ///
    /// Will omit the Content-Length header. Used for responses to certain methods (e.g., `HEAD`) or
    /// with particular status codes (e.g., 204 No Content). Consumers that read this as a body size
    /// hint are allowed to make optimizations that skip reading or writing the payload.
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
    /// Equivalent to `BodySize::Sized(0)`;
    pub const ZERO: Self = Self::Sized(0);

    /// Returns true if size hint indicates omitted or empty body.
    ///
    /// Streams will return false because it cannot be known without reading the stream.
    ///
    /// ```
    /// # use actix_http::body::BodySize;
    /// assert!(BodySize::None.is_eof());
    /// assert!(BodySize::Sized(0).is_eof());
    ///
    /// assert!(!BodySize::Sized(64).is_eof());
    /// assert!(!BodySize::Stream.is_eof());
    /// ```
    pub fn is_eof(&self) -> bool {
        matches!(self, BodySize::None | BodySize::Sized(0))
    }
}
