use actix_http::http::header::DispositionType;
use actix_web::http::Method;
use mime;

/// Describes `StaticFiles` configiration
///
/// To configure actix's static resources you need
/// to define own configiration type and implement any method
/// you wish to customize.
/// As trait implements reasonable defaults for Actix.
///
/// ## Example
///
/// ```rust,ignore
/// extern crate mime;
/// extern crate actix_web;
/// use actix_web::http::header::DispositionType;
/// use actix_web::fs::{StaticFileConfig, NamedFile};
///
/// #[derive(Default)]
/// struct MyConfig;
///
/// impl StaticFileConfig for MyConfig {
///     fn content_disposition_map(typ: mime::Name) -> DispositionType {
///         DispositionType::Attachment
///     }
/// }
///
/// let file = NamedFile::open_with_config("foo.txt", MyConfig);
/// ```
pub trait StaticFileConfig: Default {
    ///Describes mapping for mime type to content disposition header
    ///
    ///By default `IMAGE`, `TEXT` and `VIDEO` are mapped to Inline.
    ///Others are mapped to Attachment
    fn content_disposition_map(typ: mime::Name) -> DispositionType {
        match typ {
            mime::IMAGE | mime::TEXT | mime::VIDEO => DispositionType::Inline,
            _ => DispositionType::Attachment,
        }
    }

    ///Describes whether Actix should attempt to calculate `ETag`
    ///
    ///Defaults to `true`
    fn is_use_etag() -> bool {
        true
    }

    ///Describes whether Actix should use last modified date of file.
    ///
    ///Defaults to `true`
    fn is_use_last_modifier() -> bool {
        true
    }

    ///Describes allowed methods to access static resources.
    ///
    ///By default all methods are allowed
    fn is_method_allowed(_method: &Method) -> bool {
        true
    }
}

///Default content disposition as described in
///[StaticFileConfig](trait.StaticFileConfig.html)
#[derive(Default)]
pub struct DefaultConfig;

impl StaticFileConfig for DefaultConfig {}
