//! Integration test for Dart gRPC client generation.
//!
//! This test verifies that:
//! 1. The Dart gRPC client code is generated correctly
//! 2. The serializer uses proper FlatBuffers pack() pattern
//! 3. The generated code can be parsed (basic syntax check)
//!
//! Note: Actually running the Dart client against a server requires:
//! - Dart SDK
//! - flatc --dart from flatbuffers-rs
//! - grpc and flat_buffers Dart packages

#[cfg(feature = "flatbuffers")]
use std::path::Path;
#[cfg(feature = "flatbuffers")]
use tempfile::TempDir;

/// Test that compile_fbs_dart generates valid Dart gRPC client code.
#[test]
#[cfg(feature = "flatbuffers")]
fn dart_client_generation_integration() {
    let tmp = TempDir::new().unwrap();
    let fbs_dir = tmp.path().join("schema");
    std::fs::create_dir_all(&fbs_dir).unwrap();

    // Write the FlatBuffers schema
    std::fs::write(
        fbs_dir.join("greeter.fbs"),
        r#"namespace helloworld;

table HelloRequest {
    name: string;
}

table HelloReply {
    message: string;
}

rpc_service Greeter {
    SayHello(HelloRequest): HelloReply;
}
"#,
    )
    .unwrap();

    // Set OUT_DIR for grpc_build
    std::env::set_var("OUT_DIR", tmp.path().to_str().unwrap());

    // Generate Dart client code
    grpc_build::compile_fbs_dart(&[fbs_dir.join("greeter.fbs")], &[&fbs_dir], "helloworld")
        .unwrap();

    // Verify the generated file exists
    let dart_file = tmp.path().join("greeter_client.dart");
    assert!(dart_file.exists(), "should generate greeter_client.dart");

    let code = std::fs::read_to_string(&dart_file).unwrap();

    // Verify the client class is generated
    assert!(
        code.contains("class GreeterClient extends grpc.Client"),
        "should contain GreeterClient extending grpc.Client"
    );

    // Verify the RPC method is generated
    assert!(
        code.contains("say_hello"),
        "should contain say_hello method"
    );
    assert!(
        code.contains("HelloRequest"),
        "should contain HelloRequest type"
    );
    assert!(
        code.contains("HelloReply"),
        "should contain HelloReply type"
    );

    // Verify the serializer uses the pack() pattern (not UnimplementedError)
    assert!(
        code.contains("request.unpack().pack(builder)"),
        "serializer should use unpack().pack() pattern"
    );
    assert!(
        !code.contains("UnimplementedError"),
        "serializer should not throw UnimplementedError"
    );

    // Verify the deserializer works
    assert!(
        code.contains("_deserialize_HelloReply"),
        "should contain deserializer"
    );
    assert!(
        code.contains("Uint8List.fromList(responseBytes)"),
        "deserializer should handle byte conversion"
    );
}

/// Test that compile_fbs_dart handles multiple services.
#[test]
#[cfg(feature = "flatbuffers")]
fn dart_client_multiple_services() {
    let tmp = TempDir::new().unwrap();
    let fbs_dir = tmp.path().join("schema");
    std::fs::create_dir_all(&fbs_dir).unwrap();

    std::fs::write(
        fbs_dir.join("multi.fbs"),
        r#"namespace test;

table Request { id: int; }
table Response { result: string; }

rpc_service ServiceA {
    MethodA(Request): Response;
}

rpc_service ServiceB {
    MethodB(Request): Response;
}
"#,
    )
    .unwrap();

    std::env::set_var("OUT_DIR", tmp.path().to_str().unwrap());

    grpc_build::compile_fbs_dart(&[fbs_dir.join("multi.fbs")], &[&fbs_dir], "test").unwrap();

    // Should generate separate files for each service
    assert!(
        Path::new(tmp.path().join("servicea_client.dart").to_str().unwrap()).exists(),
        "should generate servicea_client.dart"
    );
    assert!(
        Path::new(tmp.path().join("serviceb_client.dart").to_str().unwrap()).exists(),
        "should generate serviceb_client.dart"
    );
}

/// Test that schema without services generates no Dart client files.
#[test]
#[cfg(feature = "flatbuffers")]
fn dart_client_no_services() {
    let tmp = TempDir::new().unwrap();
    let fbs_dir = tmp.path().join("schema");
    std::fs::create_dir_all(&fbs_dir).unwrap();

    // Schema with no rpc_service
    std::fs::write(
        fbs_dir.join("notypes.fbs"),
        r#"namespace test;

table Simple {
    value: int;
}
"#,
    )
    .unwrap();

    std::env::set_var("OUT_DIR", tmp.path().to_str().unwrap());

    // Should succeed but not generate any client files
    grpc_build::compile_fbs_dart(&[fbs_dir.join("notypes.fbs")], &[&fbs_dir], "test").unwrap();

    // No .dart files should be generated (no services)
    let files = std::fs::read_dir(tmp.path()).unwrap();
    let dart_files: Vec<_> = files
        .filter_map(|f| f.ok())
        .filter(|f| f.path().extension().unwrap_or_default() == "dart")
        .collect();

    assert!(
        dart_files.is_empty(),
        "no .dart files should be generated for schema without services"
    );
}
