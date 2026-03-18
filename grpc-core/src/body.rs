use std::{pin::Pin, task::Poll};

use http_body_util::BodyExt as _;

type BoxBody = http_body_util::combinators::UnsyncBoxBody<bytes::Bytes, crate::Status>;

/// A type-erased HTTP body used throughout the gRPC framework.
#[derive(Debug)]
pub struct Body {
    kind: Kind,
}

#[derive(Debug)]
enum Kind {
    Empty,
    Wrap(BoxBody),
}

impl Body {
    pub const fn empty() -> Self {
        Self { kind: Kind::Empty }
    }

    pub fn new<B>(body: B) -> Self
    where
        B: http_body::Body<Data = bytes::Bytes> + Send + 'static,
        B::Error: Into<crate::BoxError>,
    {
        if body.is_end_stream() {
            return Self::empty();
        }

        let body = body.map_err(crate::Status::map_error).boxed_unsync();
        Self {
            kind: Kind::Wrap(body),
        }
    }
}

impl Default for Body {
    fn default() -> Self {
        Self::empty()
    }
}

impl http_body::Body for Body {
    type Data = bytes::Bytes;
    type Error = crate::Status;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        match &mut self.kind {
            Kind::Empty => Poll::Ready(None),
            Kind::Wrap(body) => Pin::new(body).poll_frame(cx),
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

#[cfg(test)]
mod tests {
    use super::*;
    use http_body::Body as _;

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
