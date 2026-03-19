use http_body::Body as HttpBody;
use std::fmt;
use std::pin::Pin;
use std::task::Poll;

/// A Send-only type-erased body. Unlike `http_body_util::BoxBody` (which
/// requires `Send + Sync`) or `UnsyncBoxBody` (which requires neither),
/// this requires exactly `Send` — matching the bounds that tower services
/// and tokio::spawn actually need.
type SendBoxBody =
    Pin<Box<dyn HttpBody<Data = bytes::Bytes, Error = crate::Status> + Send + 'static>>;

/// A type-erased HTTP body used throughout the gRPC framework.
pub struct Body {
    kind: Kind,
}

enum Kind {
    Empty,
    Wrap(SendBoxBody),
}

impl Body {
    pub const fn empty() -> Self {
        Self { kind: Kind::Empty }
    }

    pub fn new<B>(body: B) -> Self
    where
        B: HttpBody<Data = bytes::Bytes> + Send + 'static,
        B::Error: Into<crate::BoxError>,
    {
        if body.is_end_stream() {
            return Self::empty();
        }

        use http_body_util::BodyExt as _;
        let body = body.map_err(crate::Status::map_error);
        Self {
            kind: Kind::Wrap(Box::pin(body)),
        }
    }
}

impl Default for Body {
    fn default() -> Self {
        Self::empty()
    }
}

impl HttpBody for Body {
    type Data = bytes::Bytes;
    type Error = crate::Status;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        match &mut self.kind {
            Kind::Empty => Poll::Ready(None),
            Kind::Wrap(body) => body.as_mut().poll_frame(cx),
        }
    }

    fn size_hint(&self) -> http_body::SizeHint {
        match &self.kind {
            Kind::Empty => http_body::SizeHint::with_exact(0),
            Kind::Wrap(body) => body.size_hint(),
        }
    }

    fn is_end_stream(&self) -> bool {
        match &self.kind {
            Kind::Empty => true,
            Kind::Wrap(body) => body.is_end_stream(),
        }
    }
}

impl fmt::Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            Kind::Empty => f.debug_struct("Body").field("kind", &"Empty").finish(),
            Kind::Wrap(_) => f.debug_struct("Body").field("kind", &"Wrap(..)").finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body::Body as _;

    #[test]
    fn body_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Body>();
    }

    #[test]
    fn empty_body_is_end_stream() {
        let body = Body::empty();
        assert!(body.is_end_stream());
        assert_eq!(body.size_hint().exact(), Some(0));
    }

    #[test]
    fn default_body_is_empty() {
        let body = Body::default();
        assert!(body.is_end_stream());
    }
}
