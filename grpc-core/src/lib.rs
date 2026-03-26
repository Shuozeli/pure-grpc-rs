//! Core types for the pure-grpc-rs framework.
//!
//! This crate provides the foundational types shared by both server and client:
//!
//! - [`Status`] / [`Code`] — gRPC error model with 17 status codes
//! - [`MetadataMap`] — gRPC metadata (HTTP/2 headers)
//! - [`Request`] / [`Response`] — typed wrappers with metadata and extensions
//! - [`Body`] — type-erased HTTP body
//! - [`Streaming`] — streaming message decoder
//! - Codec traits ([`codec::Codec`], [`codec::Encoder`], [`codec::Decoder`])
//! - gRPC framing (5-byte header encode/decode)
//! - Optional [`codec::prost_codec::ProstCodec`] for protobuf (feature = "prost-codec")
//! - Optional gzip compression (feature = "gzip")

pub mod body;
pub mod codec;
pub mod extensions;
pub mod http2_config;
pub mod metadata;
pub mod request;
pub mod response;
pub mod status;

pub use body::Body;
pub use codec::Streaming;
pub use extensions::GrpcMethod;
pub use metadata::MetadataMap;
pub use request::Request;
pub use response::Response;
pub use http2_config::Http2Config;
pub use status::{Code, Status};

/// A type alias for `Box<dyn std::error::Error + Send + Sync + 'static>`.
pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// A type alias for a pinned, boxed, `Send` future.
pub type BoxFuture<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'static>>;

/// A type alias for a pinned, boxed, `Send` stream.
pub type BoxStream<T> = std::pin::Pin<Box<dyn tokio_stream::Stream<Item = T> + Send + 'static>>;
