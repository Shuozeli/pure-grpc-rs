//! gRPC service code generation.
//!
//! Generates server traits and client stubs from a codec-agnostic
//! [`ir::ServiceDef`] / [`ir::MethodDef`] intermediate representation.
//!
//! Adapters convert schema-specific types into the IR:
//! - `protobuf` module (feature = "protobuf"): from protobuf-rs `ServiceDescriptorProto`
//! - `flatbuffers` module (feature = "flatbuffers"): from flatbuffers-rs `ResolvedService`

pub mod client_gen;
pub mod ir;
pub mod server_gen;

#[cfg(test)]
mod test_util;

#[cfg(feature = "protobuf")]
pub mod protobuf;

#[cfg(feature = "flatbuffers")]
pub mod flatbuffers;
