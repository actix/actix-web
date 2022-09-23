//! Reads a field into memory.
use crate::form::{FieldReader, Limits};
use crate::{Field, MultipartError};
use actix_web::HttpRequest;
use bytes::BytesMut;
use futures_core::future::LocalBoxFuture;
use futures_util::{FutureExt, TryStreamExt};
use mime::Mime;

/// Read the field into memory.
#[derive(Debug)]
pub struct Bytes {
    /// The data.
    pub data: bytes::Bytes,
    /// The value of the `content-type` header.
    pub content_type: Option<Mime>,
    /// The `filename` value in the `content-disposition` header.
    pub file_name: Option<String>,
}

impl<'t> FieldReader<'t> for Bytes {
    type Future = LocalBoxFuture<'t, Result<Self, MultipartError>>;

    fn read_field(
        _: &'t HttpRequest,
        mut field: Field,
        limits: &'t mut Limits,
    ) -> Self::Future {
        async move {
            let mut data = BytesMut::new();
            while let Some(chunk) = field.try_next().await? {
                limits.try_consume_limits(chunk.len(), true)?;
                data.extend(chunk);
            }
            Ok(Bytes {
                data: data.freeze(),
                content_type: field.content_type().map(ToOwned::to_owned),
                file_name: field
                    .content_disposition()
                    .get_filename()
                    .map(str::to_owned),
            })
        }
        .boxed_local()
    }
}
