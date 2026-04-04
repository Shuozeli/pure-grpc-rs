#[cfg(feature = "protobuf")]
mod protobuf;

#[cfg(feature = "flatbuffers")]
mod flatbuffers;

#[cfg(feature = "protobuf")]
pub use protobuf::compile_protos;

#[cfg(feature = "flatbuffers")]
pub use flatbuffers::{compile_fbs, compile_fbs_dart};
