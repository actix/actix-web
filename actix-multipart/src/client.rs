//! Multipart payload support for Actix Web Client

use bytes::Bytes;
use futures::prelude::*;

pub struct Form<'a> {
    boundary: String,
    fields: Vec<Field<'a>>,
}

/// A field in a multipart Form
pub struct Field<'a> {
    inner: FieldInner<'a>,
    content_type: String,
    content_length: Option<usize>,
}

impl<'a> Default for Form<'a> {
    fn default() -> Self {
        use rand::{distributions::Alphanumeric, thread_rng, Rng};
        let rng = thread_rng();

        Self::with_boundary(rng.sample_iter(&Alphanumeric).take(60).collect())
    }
}

impl<'a> Form<'a> {
    /// Constructs a new multipart Form with a specific boundary.
    ///
    /// If you do not want to manually construct a boundary, use `Form::default()`.
    pub fn with_boundary(boundary: String) -> Self {
        Form {
            boundary,
            fields: Vec::new(),
        }
    }

    pub fn add_stream<S>(&mut self, content_type: String, content: S)
    where
        S: Stream<Item = Result<Bytes, actix_http::Error>> + 'a,
    {
        self.fields.push(Field {
            inner: FieldInner::Stream(Box::new(content)),
            content_type,
            content_length: None, // XXX
        })
    }
}

enum FieldInner<'a> {
    Stream(Box<dyn Stream<Item = Result<Bytes, actix_http::Error>> + 'a>),
}
