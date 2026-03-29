//! gRPC client implementation built on hyper HTTP/2.
//!
//! # Key types
//!
//! - [`Channel`] — HTTP/2 connection (clone-friendly, auto-reconnects)
//! - [`Endpoint`] — builder for configuring and connecting channels
//! - [`Grpc`] — codec-agnostic dispatcher for all 4 RPC patterns
//! - [`GrpcService`] — trait alias for tower `Service` with HTTP semantics
//!
//! # Example
//!
//! ```ignore
//! let channel = Channel::connect("http://127.0.0.1:50051".parse()?).await?;
//! let mut grpc = Grpc::with_origin(channel, uri);
//! let response = grpc.unary(request, path, codec).await?;
//! ```

mod balance;
mod channel;
mod endpoint;
mod grpc;
#[cfg(feature = "h3")]
mod h3_channel;

pub use self::balance::BalancedChannel;
pub use self::channel::Channel;
pub use self::endpoint::Endpoint;
pub use self::grpc::{Grpc, GrpcService};
#[cfg(feature = "h3")]
pub use self::h3_channel::H3Channel;
