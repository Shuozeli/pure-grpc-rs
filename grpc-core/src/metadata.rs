use http::header::HeaderValue;
use http::HeaderMap;

/// HTTP Header `content-type` value for gRPC calls.
pub const GRPC_CONTENT_TYPE: HeaderValue = HeaderValue::from_static("application/grpc");

pub(crate) const GRPC_TIMEOUT_HEADER: &str = "grpc-timeout";

/// Headers reserved by the gRPC protocol that must not appear in user metadata.
const GRPC_RESERVED_HEADERS: [&str; 8] = [
    "te",
    "content-type",
    "grpc-message",
    "grpc-message-type",
    "grpc-status",
    "grpc-status-details-bin",
    "grpc-encoding",
    "grpc-accept-encoding",
];

/// A set of gRPC custom metadata entries.
///
/// Wraps `http::HeaderMap` and provides gRPC-specific operations.
#[derive(Clone, Debug, Default)]
pub struct MetadataMap {
    headers: HeaderMap,
}

impl MetadataMap {
    pub fn new() -> Self {
        MetadataMap {
            headers: HeaderMap::new(),
        }
    }

    pub fn from_headers(headers: HeaderMap) -> Self {
        MetadataMap { headers }
    }

    pub fn into_headers(self) -> HeaderMap {
        self.headers
    }

    /// Convert to headers with gRPC-reserved headers stripped.
    pub fn into_sanitized_headers(mut self) -> HeaderMap {
        for name in &GRPC_RESERVED_HEADERS {
            self.headers.remove(*name);
        }
        self.headers
    }

    /// Extend the target HeaderMap with sanitized metadata (no clone).
    pub fn extend_sanitized_into(&self, target: &mut HeaderMap) {
        for (key, value) in self.headers.iter() {
            if !GRPC_RESERVED_HEADERS.contains(&key.as_str()) {
                target.insert(key.clone(), value.clone());
            }
        }
    }

    pub fn len(&self) -> usize {
        self.headers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.headers.is_empty()
    }

    pub fn insert(
        &mut self,
        key: impl http::header::IntoHeaderName,
        value: HeaderValue,
    ) -> Option<HeaderValue> {
        self.headers.insert(key, value)
    }

    pub fn get(&self, key: &str) -> Option<&HeaderValue> {
        self.headers.get(key)
    }

    pub fn remove(&mut self, key: &str) -> Option<HeaderValue> {
        self.headers.remove(key)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.headers.contains_key(key)
    }

    /// Merge another MetadataMap into this one.
    pub fn merge(&mut self, other: MetadataMap) {
        self.headers.extend(other.headers);
    }
}

impl AsRef<HeaderMap> for MetadataMap {
    fn as_ref(&self) -> &HeaderMap {
        &self.headers
    }
}

impl AsMut<HeaderMap> for MetadataMap {
    fn as_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_empty() {
        let map = MetadataMap::new();
        assert!(map.is_empty());
        assert_eq!(map.len(), 0);
    }

    #[test]
    fn insert_and_get() {
        let mut map = MetadataMap::new();
        map.insert("x-custom", HeaderValue::from_static("value1"));
        assert_eq!(map.get("x-custom").unwrap(), "value1");
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn remove_key() {
        let mut map = MetadataMap::new();
        map.insert("x-key", HeaderValue::from_static("val"));
        assert!(map.contains_key("x-key"));
        map.remove("x-key");
        assert!(!map.contains_key("x-key"));
    }

    #[test]
    fn sanitize_removes_reserved_headers() {
        let mut map = MetadataMap::new();
        map.insert("content-type", HeaderValue::from_static("application/grpc"));
        map.insert("grpc-status", HeaderValue::from_static("0"));
        map.insert("grpc-encoding", HeaderValue::from_static("gzip"));
        map.insert("grpc-accept-encoding", HeaderValue::from_static("gzip"));
        map.insert("grpc-status-details-bin", HeaderValue::from_static("abc"));
        map.insert("x-custom", HeaderValue::from_static("keep"));

        let sanitized = map.into_sanitized_headers();
        assert!(!sanitized.contains_key("content-type"));
        assert!(!sanitized.contains_key("grpc-status"));
        assert!(!sanitized.contains_key("grpc-encoding"));
        assert!(!sanitized.contains_key("grpc-accept-encoding"));
        assert!(!sanitized.contains_key("grpc-status-details-bin"));
        assert!(sanitized.contains_key("x-custom"));
    }

    #[test]
    fn from_headers_roundtrip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-key", HeaderValue::from_static("val"));
        let map = MetadataMap::from_headers(headers);
        assert_eq!(map.get("x-key").unwrap(), "val");

        let headers = map.into_headers();
        assert_eq!(headers.get("x-key").unwrap(), "val");
    }

    #[test]
    fn merge_combines_maps() {
        let mut a = MetadataMap::new();
        a.insert("x-a", HeaderValue::from_static("1"));

        let mut b = MetadataMap::new();
        b.insert("x-b", HeaderValue::from_static("2"));

        a.merge(b);
        assert_eq!(a.len(), 2);
        assert_eq!(a.get("x-a").unwrap(), "1");
        assert_eq!(a.get("x-b").unwrap(), "2");
    }
}
