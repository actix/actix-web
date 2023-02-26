//! Reads a field into memory.

use actix_web::HttpRequest;
use bytes::BytesMut;
use futures_core::future::LocalBoxFuture;
use futures_util::TryStreamExt as _;
use mime::Mime;

use crate::{
    form::{FieldReader, Limits},
    Field, MultipartError,
};

/// Read the field into memory.
#[derive(Debug)]
pub struct Bytes {
    /// The data.
    pub data: bytes::Bytes,

    /// The value of the `Content-Type` header.
    pub content_type: Option<Mime>,

    /// The `filename` value in the `Content-Disposition` header.
    pub file_name: Option<String>,
}

impl<'t> FieldReader<'t> for Bytes {
    type Future = LocalBoxFuture<'t, Result<Self, MultipartError>>;

    fn read_field(
        _: &'t HttpRequest,
        mut field: Field,
        limits: &'t mut Limits,
    ) -> Self::Future {
        Box::pin(async move {
            let mut buf = BytesMut::with_capacity(131_072);

            while let Some(chunk) = field.try_next().await? {
                limits.try_consume_limits(chunk.len(), true)?;
                buf.extend(chunk);
            }

            Ok(Bytes {
                data: buf.freeze(),
                content_type: field.content_type().map(ToOwned::to_owned),
                file_name: field
                    .content_disposition()
                    .get_filename()
                    .map(str::to_owned),
            })
        })
    }
}
