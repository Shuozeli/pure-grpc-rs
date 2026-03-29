//! gRPC server implementation built on hyper HTTP/2.
//!
//! # Key types
//!
//! - [`Server`] — TCP listener + hyper HTTP/2 serve loop with optional TLS
//! - [`Grpc`] — codec-agnostic handler dispatching all 4 RPC patterns
//! - [`Router`] — HashMap-based service routing by path
//! - [`InterceptedService`] — wraps a service with a request interceptor
//! - Service traits: [`UnaryService`], [`ServerStreamingService`],
//!   [`ClientStreamingService`], [`StreamingService`]
//!
//! # Example
//!
//! ```ignore
//! let router = Router::new()
//!     .add_service("my.Service", my_server);
//!
//! Server::builder()
//!     .timeout(Duration::from_secs(30))
//!     .serve_with_shutdown(addr, router, ctrl_c())
//!     .await?;
//! ```

mod grpc;
#[cfg(feature = "h3")]
mod h3_server;
mod interceptor;
mod router;
mod server;
mod service;

pub use self::grpc::Grpc;
#[cfg(feature = "h3")]
pub use self::h3_server::H3Server;
pub use self::interceptor::{InterceptedService, Interceptor};
pub use self::router::Router;
pub use self::server::Server;
pub use self::service::{
    ClientStreamingService, NamedService, ServerStreamingService, StreamingService, UnaryService,
};
