//! gRPC-Web protocol translation service.

use crate::call::GrpcWebCall;
use crate::{is_grpc_web, Encoding, GRPC};
use bytes::Bytes;
use http::{header, HeaderValue, Request, Response};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower_service::Service;

/// A tower `Service` that translates between gRPC-Web and native gRPC.
///
/// Wraps an inner gRPC service. For gRPC-Web requests, the request is
/// translated to standard gRPC before forwarding, and the response is
/// translated back to gRPC-Web format. Standard gRPC requests pass through
/// unchanged.
#[derive(Clone)]
pub struct GrpcWebService<S> {
    inner: S,
}

impl<S> GrpcWebService<S> {
    pub fn new(inner: S) -> Self {
        Self { inner }
    }
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for GrpcWebService<S>
where
    S: Service<Request<GrpcWebCall<ReqBody>>, Response = Response<ResBody>>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    S::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    ReqBody: http_body::Body<Data = Bytes> + Unpin + Send + 'static,
    ResBody: http_body::Body<Data = Bytes> + Unpin + Send + 'static,
    ResBody::Error: std::fmt::Display,
{
    type Response = Response<GrpcWebCall<ResBody>>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let content_type = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if !is_grpc_web(content_type) {
            // Not a gRPC-Web request — pass through with no-op wrappers
            let (parts, body) = req.into_parts();
            let wrapped_body = GrpcWebCall::request(body, Encoding::Binary);
            let req = Request::from_parts(parts, wrapped_body);
            let mut inner = self.inner.clone();
            return Box::pin(async move {
                let resp = inner.call(req).await?;
                let (parts, body) = resp.into_parts();
                Ok(Response::from_parts(
                    parts,
                    GrpcWebCall::response(body, Encoding::Binary),
                ))
            });
        }

        let encoding = Encoding::from_content_type(content_type);

        // Coerce gRPC-Web request into standard gRPC
        let (mut parts, body) = req.into_parts();

        // Rewrite content-type
        parts
            .headers
            .insert(header::CONTENT_TYPE, HeaderValue::from_static(GRPC));

        // Remove content-length (body will change due to base64 decoding)
        parts.headers.remove(header::CONTENT_LENGTH);

        // Add TE: trailers (required by gRPC over HTTP/2)
        parts
            .headers
            .insert("te", HeaderValue::from_static("trailers"));

        let wrapped_body = GrpcWebCall::request(body, encoding);
        let req = Request::from_parts(parts, wrapped_body);

        let mut inner = self.inner.clone();

        Box::pin(async move {
            let resp = inner.call(req).await?;
            let (mut parts, body) = resp.into_parts();

            // Set gRPC-Web content type on response
            let response_ct = match encoding {
                Encoding::Base64 => crate::GRPC_WEB_TEXT,
                Encoding::Binary => crate::GRPC_WEB,
            };
            parts
                .headers
                .insert(header::CONTENT_TYPE, HeaderValue::from_static(response_ct));

            // Remove content-length (response body will change)
            parts.headers.remove(header::CONTENT_LENGTH);

            Ok(Response::from_parts(
                parts,
                GrpcWebCall::response(body, encoding),
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::Empty;

    #[test]
    fn grpc_web_detection() {
        assert!(is_grpc_web("application/grpc-web"));
        assert!(is_grpc_web("application/grpc-web+proto"));
        assert!(is_grpc_web("application/grpc-web-text"));
        assert!(is_grpc_web("application/grpc-web-text+proto"));
        assert!(!is_grpc_web("application/grpc"));
        assert!(!is_grpc_web("application/json"));
    }

    #[test]
    fn encoding_detection() {
        assert_eq!(
            Encoding::from_content_type("application/grpc-web"),
            Encoding::Binary
        );
        assert_eq!(
            Encoding::from_content_type("application/grpc-web-text"),
            Encoding::Base64
        );
        assert_eq!(
            Encoding::from_content_type("application/grpc-web-text+proto"),
            Encoding::Base64
        );
    }

    // Service type compatibility test
    fn _assert_service_bounds<S>()
    where
        S: Service<
                Request<GrpcWebCall<Empty<Bytes>>>,
                Response = Response<Empty<Bytes>>,
                Error = std::convert::Infallible,
            > + Clone,
        S::Future: Send + 'static,
    {
        // Just a compile-time check — GrpcWebService wraps S
        let _: GrpcWebService<S>;
    }
}
