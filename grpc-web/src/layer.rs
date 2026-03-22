//! Tower layer for gRPC-Web protocol translation.

use crate::service::GrpcWebService;
use tower_layer::Layer;

/// A tower `Layer` that wraps services with gRPC-Web protocol support.
///
/// Transparently translates between gRPC-Web and native gRPC protocols.
/// Non-gRPC-Web requests (standard gRPC over HTTP/2) pass through unchanged.
#[derive(Debug, Clone, Copy)]
pub struct GrpcWebLayer;

impl GrpcWebLayer {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GrpcWebLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> Layer<S> for GrpcWebLayer {
    type Service = GrpcWebService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        GrpcWebService::new(inner)
    }
}
