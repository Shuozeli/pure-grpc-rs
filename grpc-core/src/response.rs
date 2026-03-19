use crate::metadata::MetadataMap;
use http::Extensions;

/// A gRPC response and metadata from an RPC call.
#[derive(Debug)]
pub struct Response<T> {
    metadata: MetadataMap,
    message: T,
    extensions: Extensions,
}

impl<T> Response<T> {
    pub fn new(message: T) -> Self {
        Response {
            metadata: MetadataMap::new(),
            message,
            extensions: Extensions::new(),
        }
    }

    pub fn get_ref(&self) -> &T {
        &self.message
    }

    pub fn get_mut(&mut self) -> &mut T {
        &mut self.message
    }

    pub fn metadata(&self) -> &MetadataMap {
        &self.metadata
    }

    pub fn metadata_mut(&mut self) -> &mut MetadataMap {
        &mut self.metadata
    }

    pub fn into_inner(self) -> T {
        self.message
    }

    pub fn into_parts(self) -> (MetadataMap, T, Extensions) {
        (self.metadata, self.message, self.extensions)
    }

    pub fn from_parts(metadata: MetadataMap, message: T, extensions: Extensions) -> Self {
        Self {
            metadata,
            message,
            extensions,
        }
    }

    pub fn from_http(res: http::Response<T>) -> Self {
        let (head, message) = res.into_parts();
        Self::from_http_parts(head, message)
    }

    /// Construct from pre-split HTTP response parts and a message body.
    pub fn from_http_parts(parts: http::response::Parts, message: T) -> Self {
        Response {
            metadata: MetadataMap::from_headers(parts.headers),
            message,
            extensions: parts.extensions,
        }
    }

    pub fn into_http(self) -> http::Response<T> {
        let mut res = http::Response::new(self.message);
        *res.version_mut() = http::Version::HTTP_2;
        *res.headers_mut() = self.metadata.into_sanitized_headers();
        *res.extensions_mut() = self.extensions;
        res
    }

    pub fn map<F, U>(self, f: F) -> Response<U>
    where
        F: FnOnce(T) -> U,
    {
        Response {
            metadata: self.metadata,
            message: f(self.message),
            extensions: self.extensions,
        }
    }

    pub fn extensions(&self) -> &Extensions {
        &self.extensions
    }

    pub fn extensions_mut(&mut self) -> &mut Extensions {
        &mut self.extensions
    }
}

impl<T> From<T> for Response<T> {
    fn from(inner: T) -> Self {
        Response::new(inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_new() {
        let resp = Response::new("hello");
        assert_eq!(resp.get_ref(), &"hello");
        assert!(resp.metadata().is_empty());
    }

    #[test]
    fn response_into_parts_and_from_parts() {
        let resp = Response::new(42);
        let (meta, msg, ext) = resp.into_parts();
        assert_eq!(msg, 42);

        let resp2 = Response::from_parts(meta, msg, ext);
        assert_eq!(*resp2.get_ref(), 42);
    }

    #[test]
    fn response_map() {
        let resp = Response::new(5);
        let resp2 = resp.map(|n| n.to_string());
        assert_eq!(resp2.get_ref(), "5");
    }

    #[test]
    fn response_from_value() {
        let resp: Response<i32> = 42.into();
        assert_eq!(*resp.get_ref(), 42);
    }

    #[test]
    fn response_from_http_parts() {
        let http_resp = http::Response::builder()
            .header("x-custom", "value")
            .body("payload")
            .unwrap();
        let (parts, body) = http_resp.into_parts();
        let resp = Response::from_http_parts(parts, body);
        assert_eq!(resp.get_ref(), &"payload");
        assert!(resp.metadata().get("x-custom").is_some());
    }
}
