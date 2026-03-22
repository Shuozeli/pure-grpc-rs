//! Rich error detail types for gRPC (google.rpc.*).
//!
//! Provides typed wrappers for the standard gRPC error detail messages
//! (`ErrorInfo`, `RetryInfo`, `BadRequest`, etc.) and the `StatusExt`
//! trait to attach/extract them from `grpc_core::Status`.
//!
//! Requires the `prost-codec` feature (enabled by default).
//!
//! # Usage
//!
//! ```ignore
//! use grpc_core::status::{Code, Status};
//! use grpc_types::{ErrorInfo, StatusExt};
//!
//! // Create a Status with rich error details
//! let status = Status::with_error_details(
//!     Code::InvalidArgument,
//!     "invalid request",
//!     vec![ErrorInfo {
//!         reason: "FIELD_REQUIRED".into(),
//!         domain: "example.com".into(),
//!         metadata: Default::default(),
//!     }.into()],
//! );
//!
//! // Extract details from a Status
//! let details = status.error_details().unwrap();
//! ```

#[cfg(not(feature = "prost-codec"))]
compile_error!(
    "grpc-types requires the `prost-codec` feature. \
     Enable it with: grpc-types = { features = [\"prost-codec\"] }"
);

#[cfg(feature = "prost-codec")]
mod any_ext;
#[cfg(feature = "prost-codec")]
mod status_ext;

// The generated code uses absolute paths like `google::protobuf::Any`,
// `google::protobuf::Duration`, and `google::rpc::LocalizedMessage`.
// In Rust 2021, these resolve from the crate root. We provide a `google`
// module at the crate root that maps these paths:
//   google::protobuf::Any      -> prost_types::Any
//   google::protobuf::Duration -> prost_types::Duration
//   google::rpc::*             -> types defined in the include below
//
// The generated types themselves are included directly in google::rpc,
// with a private shadow `google` module inside to handle self-references
// from nested submodules (e.g., `google::rpc::LocalizedMessage` from
// within `bad_request::FieldViolation`).
#[cfg(feature = "prost-codec")]
#[allow(clippy::all, non_camel_case_types, missing_docs)]
pub mod google {
    pub mod protobuf {
        pub use prost_types::Any;
        pub use prost_types::Duration;
    }
    pub mod rpc {
        // Shadow `google` so that nested submodules (e.g., `bad_request`)
        // can resolve `google::protobuf::Any` and `google::rpc::LocalizedMessage`.
        // Rust checks current module scope first, so this shadow takes priority
        // over the crate-root `google`.
        #[doc(hidden)]
        pub mod google {
            pub mod protobuf {
                pub use prost_types::Any;
                pub use prost_types::Duration;
            }
            pub mod rpc {
                // Re-export the parent `rpc` module's items. This is NOT circular
                // because the generated types are defined in this same `rpc` module
                // (via include!), and Rust resolves `use super::super::*` by looking
                // at all items defined in the parent, including the included ones.
                pub use super::super::LocalizedMessage;
            }
        }
        include!(concat!(env!("OUT_DIR"), "/google.rpc.rs"));
    }
}

// Re-export all google.rpc types at the crate root for convenience.
#[cfg(feature = "prost-codec")]
pub use google::rpc::*;

/// The `google.rpc.Status` protobuf message, re-exported as `RpcStatus`
/// to avoid collision with `grpc_core::Status`.
#[cfg(feature = "prost-codec")]
pub use google::rpc::Status as RpcStatus;

#[cfg(feature = "prost-codec")]
pub use any_ext::{decode_any, ErrorDetail, FromAny, IntoAny};
#[cfg(feature = "prost-codec")]
pub use status_ext::StatusExt;
