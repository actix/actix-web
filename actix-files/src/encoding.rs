use mime::Mime;

/// Transforms MIME `text/*` types into their UTF-8 equivalent, if supported.
///
/// MIME types that are converted
/// - application/javascript
/// - text/html
/// - text/css
/// - text/plain
/// - text/csv
/// - text/tab-separated-values
pub(crate) fn equiv_utf8_text(ct: Mime) -> Mime {
    // use (roughly) order of file-type popularity for a web server

    if ct == mime::APPLICATION_JAVASCRIPT {
        return mime::APPLICATION_JAVASCRIPT_UTF_8;
    }

    if ct == mime::TEXT_HTML {
        return mime::TEXT_HTML_UTF_8;
    }

    if ct == mime::TEXT_CSS {
        return mime::TEXT_CSS_UTF_8;
    }

    if ct == mime::TEXT_PLAIN {
        return mime::TEXT_PLAIN_UTF_8;
    }

    if ct == mime::TEXT_CSV {
        return mime::TEXT_CSV_UTF_8;
    }

    if ct == mime::TEXT_TAB_SEPARATED_VALUES {
        return mime::TEXT_TAB_SEPARATED_VALUES_UTF_8;
    }

    ct
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_equiv_utf8_text() {
        assert_eq!(equiv_utf8_text(mime::TEXT_PLAIN), mime::TEXT_PLAIN_UTF_8);
        assert_eq!(equiv_utf8_text(mime::TEXT_XML), mime::TEXT_XML);
        assert_eq!(equiv_utf8_text(mime::IMAGE_PNG), mime::IMAGE_PNG);
    }
}
