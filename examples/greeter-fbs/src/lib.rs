//! FlatBuffers greeter -- generated types and gRPC stubs from
//! `schema/greeter.fbs`. The build script runs `grpc_build::compile_fbs`,
//! which writes both the FlatBuffers data types and the FlatBuffers gRPC
//! glue (owned-wrapper aliases, `FlatBufferGrpcMessage` impls, server +
//! client modules) into `OUT_DIR`.

// Include the generated FlatBuffers readers/builders/Object API types.
#[allow(
    unused_imports,
    dead_code,
    non_snake_case,
    clippy::extra_unused_lifetimes,
    clippy::needless_lifetimes,
    clippy::derivable_impls,
    clippy::unnecessary_cast
)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/greeter_generated.rs"));
}

// Include the generated gRPC stubs at the top level: this brings in the
// `HelloRequest` / `HelloReply` aliases, `FlatBufferGrpcMessage` impls,
// `greeter_server` module, and `greeter_client` module.
include!(concat!(env!("OUT_DIR"), "/greeter_grpc.rs"));

#[cfg(test)]
mod tests {
    use super::*;
    use grpc_codec_flatbuffers::FlatBufferGrpcMessage;

    // The owned wrappers are `#[non_exhaustive]`, so cross-crate users
    // build them with `Default + field assignment`. Inside this crate the
    // restriction does not apply, and clippy nudges us toward struct
    // literal -- so the tests use struct literal here while the example
    // bins (separate crates) demonstrate the cross-crate idiom.

    #[test]
    fn hello_request_roundtrip() {
        // Arrange: schema declares `name: string;` (optional in FlatBuffers
        // unless marked `(required)`), so the Object API surfaces it as
        // Option<String>.
        let req = HelloRequest {
            name: Some("FlatBuffers".into()),
        };

        // Act
        let decoded = HelloRequest::decode_flatbuffer(&req.encode_flatbuffer()).unwrap();

        // Assert
        assert_eq!(decoded.name.as_deref(), Some("FlatBuffers"));
    }

    #[test]
    fn hello_reply_roundtrip() {
        // Arrange
        let reply = HelloReply {
            message: Some("Hello!".into()),
        };

        // Act
        let decoded = HelloReply::decode_flatbuffer(&reply.encode_flatbuffer()).unwrap();

        // Assert
        assert_eq!(decoded.message.as_deref(), Some("Hello!"));
    }

    #[test]
    fn decode_invalid_data() {
        // Arrange / Act
        let result = HelloRequest::decode_flatbuffer(&[0, 0]);

        // Assert
        assert!(result.is_err());
    }
}
