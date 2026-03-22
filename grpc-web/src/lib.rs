//! gRPC-Web protocol translation layer.
//!
//! Enables browser clients to call gRPC services over HTTP/1.1 using the
//! [gRPC-Web protocol](https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-WEB.md).
//!
//! Supports:
//! - `application/grpc-web` (binary mode)
//! - `application/grpc-web+proto` (binary mode, protobuf)
//! - `application/grpc-web-text` (base64 mode)
//! - `application/grpc-web-text+proto` (base64 mode, protobuf)
//!
//! Only **unary** and **server-streaming** RPCs are supported (gRPC-Web spec limitation).
//!
//! # Usage
//!
//! ```ignore
//! use grpc_web::GrpcWebLayer;
//!
//! // Wrap your gRPC router with the gRPC-Web layer
//! let layered = router.into_layered(GrpcWebLayer::new());
//!
//! // Serve on HTTP/1.1 + HTTP/2
//! Server::builder().serve(addr, layered).await?;
//! ```

mod call;
mod layer;
mod service;

pub use layer::GrpcWebLayer;
pub use service::GrpcWebService;

/// gRPC-Web content types.
const GRPC_WEB: &str = "application/grpc-web";
const GRPC_WEB_PROTO: &str = "application/grpc-web+proto";
const GRPC_WEB_TEXT: &str = "application/grpc-web-text";
const GRPC_WEB_TEXT_PROTO: &str = "application/grpc-web-text+proto";

/// Standard gRPC content type.
const GRPC: &str = "application/grpc";

/// Size of gRPC frame header (1 byte flag + 4 bytes u32 length).
const GRPC_HEADER_SIZE: usize = 5;

/// Check if the content type indicates a gRPC-Web request.
fn is_grpc_web(content_type: &str) -> bool {
    content_type == GRPC_WEB
        || content_type == GRPC_WEB_PROTO
        || content_type == GRPC_WEB_TEXT
        || content_type == GRPC_WEB_TEXT_PROTO
}

/// Encoding mode determined by content type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Encoding {
    /// Binary mode — frames are raw gRPC frames.
    Binary,
    /// Base64 mode — frames are base64-encoded.
    Base64,
}

impl Encoding {
    fn from_content_type(ct: &str) -> Self {
        if ct.contains("text") {
            Encoding::Base64
        } else {
            Encoding::Binary
        }
    }
}
