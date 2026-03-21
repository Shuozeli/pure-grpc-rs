use crate::metadata::{MetadataMap, GRPC_TIMEOUT_HEADER};
use http::Extensions;
use std::time::Duration;
use tokio_stream::Stream;

/// A gRPC request and metadata from an RPC call.
#[derive(Debug)]
pub struct Request<T> {
    metadata: MetadataMap,
    message: T,
    extensions: Extensions,
}

/// Trait for types that can be converted into a `Request<T>`.
pub trait IntoRequest<T>: sealed::Sealed {
    fn into_request(self) -> Request<T>;
}

/// Trait for types that can be converted into a streaming `Request<S>`.
pub trait IntoStreamingRequest: sealed::Sealed {
    type Stream: Stream<Item = Self::Message> + Send + 'static;
    type Message;
    fn into_streaming_request(self) -> Request<Self::Stream>;
}

impl<T> Request<T> {
    pub fn new(message: T) -> Self {
        Request {
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

    pub fn into_parts(self) -> (MetadataMap, Extensions, T) {
        (self.metadata, self.extensions, self.message)
    }

    pub fn from_parts(metadata: MetadataMap, extensions: Extensions, message: T) -> Self {
        Self {
            metadata,
            extensions,
            message,
        }
    }

    pub fn from_http(http: http::Request<T>) -> Self {
        let (parts, message) = http.into_parts();
        Request {
            metadata: MetadataMap::from_headers(parts.headers),
            message,
            extensions: parts.extensions,
        }
    }

    pub fn from_http_parts(parts: http::request::Parts, message: T) -> Self {
        Request {
            metadata: MetadataMap::from_headers(parts.headers),
            message,
            extensions: parts.extensions,
        }
    }

    pub fn into_http(
        self,
        uri: http::Uri,
        method: http::Method,
        version: http::Version,
        sanitize: bool,
    ) -> http::Request<T> {
        let mut request = http::Request::new(self.message);
        *request.version_mut() = version;
        *request.method_mut() = method;
        *request.uri_mut() = uri;
        *request.headers_mut() = if sanitize {
            self.metadata.into_sanitized_headers()
        } else {
            self.metadata.into_headers()
        };
        *request.extensions_mut() = self.extensions;
        request
    }

    pub fn map<F, U>(self, f: F) -> Request<U>
    where
        F: FnOnce(T) -> U,
    {
        Request {
            metadata: self.metadata,
            message: f(self.message),
            extensions: self.extensions,
        }
    }

    /// Set the max duration the request is allowed to take.
    pub fn set_timeout(&mut self, deadline: Duration) {
        let value: http::HeaderValue = duration_to_grpc_timeout(deadline)
            .parse()
            .expect("grpc-timeout value is always valid ASCII digits + unit char");
        self.metadata_mut().insert(GRPC_TIMEOUT_HEADER, value);
    }

    pub fn extensions(&self) -> &Extensions {
        &self.extensions
    }

    pub fn extensions_mut(&mut self) -> &mut Extensions {
        &mut self.extensions
    }
}

// --- IntoRequest blanket impls ---

impl<T> IntoRequest<T> for T {
    fn into_request(self) -> Request<Self> {
        Request::new(self)
    }
}

impl<T> IntoRequest<T> for Request<T> {
    fn into_request(self) -> Request<T> {
        self
    }
}

impl<T> IntoStreamingRequest for T
where
    T: Stream + Send + 'static,
{
    type Stream = T;
    type Message = T::Item;
    fn into_streaming_request(self) -> Request<Self> {
        Request::new(self)
    }
}

impl<T> IntoStreamingRequest for Request<T>
where
    T: Stream + Send + 'static,
{
    type Stream = T;
    type Message = T::Item;
    fn into_streaming_request(self) -> Self {
        self
    }
}

impl<T> sealed::Sealed for T {}

mod sealed {
    pub trait Sealed {}
}

fn duration_to_grpc_timeout(duration: Duration) -> String {
    fn try_format<V: Into<u128>>(
        duration: Duration,
        unit: char,
        convert: impl FnOnce(Duration) -> V,
    ) -> Option<String> {
        let max_size: u128 = 99_999_999;
        let value = convert(duration).into();
        if value > max_size {
            None
        } else {
            Some(format!("{value}{unit}"))
        }
    }

    try_format(duration, 'n', |d| d.as_nanos())
        .or_else(|| try_format(duration, 'u', |d| d.as_micros()))
        .or_else(|| try_format(duration, 'm', |d| d.as_millis()))
        .or_else(|| try_format(duration, 'S', |d| d.as_secs()))
        .or_else(|| try_format(duration, 'M', |d| d.as_secs() / 60))
        .or_else(|| {
            try_format(duration, 'H', |d| {
                let minutes = d.as_secs() / 60;
                minutes / 60
            })
        })
        .expect("duration is unrealistically large")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_new_has_empty_metadata() {
        let req = Request::new(42);
        assert_eq!(*req.get_ref(), 42);
        assert!(req.metadata().is_empty());
    }

    #[test]
    fn request_into_parts_and_from_parts() {
        let mut req = Request::new("hello");
        req.metadata_mut()
            .insert("x-key", http::HeaderValue::from_static("val"));
        let (meta, ext, msg) = req.into_parts();
        assert_eq!(msg, "hello");

        let req2 = Request::from_parts(meta, ext, msg);
        assert_eq!(req2.get_ref(), &"hello");
        assert_eq!(req2.metadata().get("x-key").unwrap(), "val");
    }

    #[test]
    fn request_map_transforms_message() {
        let req = Request::new(5);
        let req2 = req.map(|n| n * 2);
        assert_eq!(*req2.get_ref(), 10);
    }

    #[test]
    fn set_timeout_encodes_grpc_timeout() {
        let mut req = Request::new(());
        req.set_timeout(Duration::from_secs(30));
        let val = req
            .metadata()
            .get("grpc-timeout")
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(val, "30000000u");
    }

    #[test]
    fn into_request_for_plain_value() {
        let req: Request<i32> = 42.into_request();
        assert_eq!(*req.get_ref(), 42);
    }

    #[test]
    fn into_request_for_request_is_identity() {
        let req = Request::new(42);
        let req2: Request<i32> = req.into_request();
        assert_eq!(*req2.get_ref(), 42);
    }

    #[test]
    fn timeout_less_than_second() {
        let timeout = Duration::from_millis(500);
        let value = duration_to_grpc_timeout(timeout);
        assert_eq!(value, format!("{}u", timeout.as_micros()));
    }

    #[test]
    fn timeout_very_long() {
        let one_hour = Duration::from_secs(3600);
        let value = duration_to_grpc_timeout(one_hour);
        assert_eq!(value, format!("{}m", one_hour.as_millis()));
    }
}
