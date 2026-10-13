//! Private HTTP head and error adapters for the HTTP/2 backend.

use crate::{
    error::{DispatchError, ParseError, PayloadError},
    header, RequestHead, Version,
};

pub(super) fn payload_error(err: h2_04::Error) -> PayloadError {
    #[cfg(feature = "http2")]
    {
        super::legacy::error(err).into()
    }

    #[cfg(not(feature = "http2"))]
    {
        PayloadError::Http2(crate::error::Http2Error::from_backend(err))
    }
}

pub(super) fn dispatch_error(err: h2_04::Error) -> DispatchError {
    #[cfg(feature = "http2")]
    {
        super::legacy::error(err).into()
    }

    #[cfg(not(feature = "http2"))]
    {
        DispatchError::Http2(crate::error::Http2Error::from_backend(err))
    }
}

pub(super) fn request_head(
    parts: http_1::request::Parts,
    head: &mut RequestHead,
) -> Result<(), ParseError> {
    head.method = parts
        .method
        .as_str()
        .parse()
        .map_err(|_| ParseError::Method)?;
    head.uri = parts.uri.to_string().parse().map_err(ParseError::Uri)?;
    head.version = Version::HTTP_2;

    for (name, value) in &parts.headers {
        let name = header::HeaderName::from_bytes(name.as_str().as_bytes())
            .map_err(|_| ParseError::Header)?;
        let mut converted =
            header::HeaderValue::from_bytes(value.as_bytes()).map_err(|_| ParseError::Header)?;
        converted.set_sensitive(value.is_sensitive());
        head.headers.append(name, converted);
    }

    Ok(())
}

pub(super) fn response_head(response: http::Response<()>) -> http_1::Response<()> {
    let (parts, ()) = response.into_parts();
    let mut response = http_1::Response::new(());
    *response.status_mut() = http_1::StatusCode::from_u16(parts.status.as_u16())
        .expect("http 0.2 and 1.x accept the same status codes");
    *response.version_mut() = http_1::Version::HTTP_2;

    for (name, value) in &parts.headers {
        let name = http_1::header::HeaderName::from_bytes(name.as_str().as_bytes())
            .expect("http 0.2 header names are valid in http 1.x");
        let mut converted = http_1::HeaderValue::from_bytes(value.as_bytes())
            .expect("http 0.2 header values are valid in http 1.x");
        converted.set_sensitive(value.is_sensitive());
        response.headers_mut().append(name, converted);
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_headers_retain_duplicates_bytes_and_sensitivity() {
        let mut request = http_1::Request::builder()
            .method("CUSTOM")
            .uri("https://example.test/a%2Fb?q=%FF")
            .header("x-repeat", "one")
            .header("x-repeat", "two")
            .body(())
            .unwrap();
        let mut value = http_1::HeaderValue::from_bytes(b"\x80secret").unwrap();
        value.set_sensitive(true);
        request.headers_mut().insert("x-private", value);
        let mut head = RequestHead::default();

        request_head(request.into_parts().0, &mut head).unwrap();

        assert_eq!(head.method.as_str(), "CUSTOM");
        assert_eq!(head.uri, "https://example.test/a%2Fb?q=%FF");
        assert_eq!(head.version, Version::HTTP_2);
        assert_eq!(
            head.headers.get_all("x-repeat").collect::<Vec<_>>(),
            ["one", "two"]
        );
        assert_eq!(
            head.headers.get("x-private").unwrap().as_bytes(),
            b"\x80secret"
        );
        assert!(head.headers.get("x-private").unwrap().is_sensitive());
    }

    #[test]
    fn response_headers_retain_duplicates_bytes_and_sensitivity() {
        let mut response = http::Response::builder()
            .status(201)
            .header("set-cookie", "a=1")
            .header("set-cookie", "b=2")
            .body(())
            .unwrap();
        let mut value = http::HeaderValue::from_bytes(b"\x80secret").unwrap();
        value.set_sensitive(true);
        response.headers_mut().insert("x-private", value);

        let converted = response_head(response);

        assert_eq!(converted.status(), 201);
        assert_eq!(converted.version(), http_1::Version::HTTP_2);
        assert_eq!(
            converted
                .headers()
                .get_all("set-cookie")
                .iter()
                .collect::<Vec<_>>(),
            ["a=1", "b=2"]
        );
        assert_eq!(converted.headers()["x-private"].as_bytes(), b"\x80secret");
        assert!(converted.headers()["x-private"].is_sensitive());
    }
}
