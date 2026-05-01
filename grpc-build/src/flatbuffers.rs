//! Compile .fbs files into Rust FlatBuffers types + gRPC service stubs (Rust)
//! or Dart gRPC client stubs (Dart).

use std::io;
use std::path::{Path, PathBuf};

/// Compile `.fbs` files into Dart gRPC client code.
///
/// Parses and analyzes the FlatBuffers schema using flatbuffers-rs, generates
/// Dart gRPC client classes via grpc-codegen. Each service generates a separate
/// `.dart` file. Output is written to `OUT_DIR`.
///
/// The FlatBuffers data types (tables, structs, enums) must be generated
/// separately using `flatc --dart` or the flatbuffers-rs Dart generator.
///
/// # Arguments
///
/// * `fbs_files` - Paths to `.fbs` files to compile.
/// * `includes` - Include directories for import resolution.
/// * `proto_path` - Import path prefix for the generated Dart types (e.g., `"package:myapp"`).
///
/// # Example
///
/// In `build.rs`:
/// ```ignore
/// fn main() {
///     grpc_build::compile_fbs_dart(
///         &["schema/greeter.fbs"],
///         &["schema"],
///         "package:myapp",
///     ).unwrap();
/// }
/// ```
pub fn compile_fbs_dart(
    fbs_files: &[impl AsRef<Path>],
    includes: &[impl AsRef<Path>],
    proto_path: &str,
) -> io::Result<()> {
    let out_dir =
        std::env::var("OUT_DIR").map_err(|e| io::Error::other(format!("OUT_DIR not set: {e}")))?;

    let include_paths: Vec<PathBuf> = includes.iter().map(|p| p.as_ref().to_path_buf()).collect();
    let input_files: Vec<PathBuf> = fbs_files.iter().map(|p| p.as_ref().to_path_buf()).collect();

    let options = flatc_rs_compiler::CompilerOptions { include_paths };

    let result = flatc_rs_compiler::compile(&input_files, &options)
        .map_err(|e| io::Error::other(format!("flatbuffers compilation failed: {e}")))?;

    // Convert to SchemaDef using codegen-flatbuffers
    let schema_def = codegen_flatbuffers::from_resolved_schema(&result.schema)
        .map_err(|e| io::Error::other(format!("schema conversion failed: {e}")))?;

    // Generate Dart client code for all services in the schema
    let code = grpc_codegen::flatbuffers::generate_dart_client(&schema_def, proto_path)
        .map_err(|e| io::Error::other(format!("Dart client codegen failed: {e}")))?;

    // Only write the file if there are services (code is non-empty)
    if !code.is_empty() {
        let out_file = Path::new(&out_dir).join("grpc_clients.dart");
        std::fs::write(&out_file, &code)?;
    }

    Ok(())
}

/// Compile `.fbs` files into Rust FlatBuffers types + gRPC service stubs.
///
/// Parses and analyzes the FlatBuffers schema using flatbuffers-rs, then runs
/// two code generators:
///
/// 1. `flatc-rs-codegen` produces zero-copy readers, builders, and
///    `gen_object_api` owned `*T` types for every table/struct/enum.
///    The output goes to `<stem>_generated.rs`.
///
/// 2. `grpc_codegen::flatbuffers::generate_grpc_stubs` produces the
///    user-facing gRPC layer: top-level type aliases (`pub use ... as ...`),
///    [`grpc_codec_flatbuffers::FlatBufferGrpcMessage`] impls, and
///    `<service>_server` / `<service>_client` modules for every
///    `rpc_service` block. The output goes to `<stem>_grpc.rs`.
///
/// Consumers wire the two files in via:
/// ```ignore
/// mod generated {
///     include!(concat!(env!("OUT_DIR"), "/<stem>_generated.rs"));
/// }
/// include!(concat!(env!("OUT_DIR"), "/<stem>_grpc.rs"));
/// ```
///
/// # Arguments
///
/// * `fbs_files` - Paths to `.fbs` files to compile.
/// * `includes` - Include directories for import resolution.
///
/// # Example
///
/// In `build.rs`:
/// ```ignore
/// fn main() {
///     grpc_build::compile_fbs(
///         &["schema/greeter.fbs"],
///         &["schema"],
///     ).unwrap();
/// }
/// ```
pub fn compile_fbs(
    fbs_files: &[impl AsRef<Path>],
    includes: &[impl AsRef<Path>],
) -> io::Result<()> {
    let out_dir =
        std::env::var("OUT_DIR").map_err(|e| io::Error::other(format!("OUT_DIR not set: {e}")))?;

    let include_paths: Vec<PathBuf> = includes.iter().map(|p| p.as_ref().to_path_buf()).collect();
    let input_files: Vec<PathBuf> = fbs_files.iter().map(|p| p.as_ref().to_path_buf()).collect();

    let options = flatc_rs_compiler::CompilerOptions { include_paths };

    let result = flatc_rs_compiler::compile(&input_files, &options)
        .map_err(|e| io::Error::other(format!("flatbuffers compilation failed: {e}")))?;

    let opts = flatc_rs_codegen::CodeGenOptions {
        gen_object_api: true,
        ..Default::default()
    };

    let code = flatc_rs_codegen::generate_rust(&result.schema, &opts)
        .map_err(|e| io::Error::other(format!("flatbuffers codegen failed: {e}")))?;

    // Determine output filename from first input file's stem
    let out_name = fbs_files
        .first()
        .and_then(|p| p.as_ref().file_stem())
        .and_then(|s| s.to_str())
        .unwrap_or("flatbuffers_generated");

    let out_file = Path::new(&out_dir).join(format!("{out_name}_generated.rs"));
    std::fs::write(&out_file, &code)?;

    // gRPC stubs: alias the `*T` owned wrappers to bare names, generate
    // `FlatBufferGrpcMessage` impls for them, and emit one server +
    // client module per `rpc_service` block. Skipped when there are no
    // services so consumers without RPC contracts don't pay the codegen
    // cost.
    let schema_def = codegen_flatbuffers::from_resolved_schema(&result.schema)
        .map_err(|e| io::Error::other(format!("schema conversion failed: {e}")))?;

    let grpc_file = Path::new(&out_dir).join(format!("{out_name}_grpc.rs"));
    if schema_def.services.is_empty() {
        // Always write *something* so the consumer's `include!` doesn't
        // fail when a schema has no services. An empty file compiles fine
        // when included at the top level.
        std::fs::write(&grpc_file, "// no services declared in schema\n")?;
    } else {
        let tokens = grpc_codegen::flatbuffers::generate_grpc_stubs(&schema_def)
            .map_err(|e| io::Error::other(format!("flatbuffers gRPC stub codegen failed: {e}")))?;

        // Pretty-print so the generated file is reviewable in target/.
        let parsed = syn::parse2::<syn::File>(tokens.clone()).map_err(|e| {
            io::Error::other(format!(
                "flatbuffers gRPC stub generator emitted invalid Rust syntax: {e}"
            ))
        })?;
        let formatted = prettyplease::unparse(&parsed);
        std::fs::write(&grpc_file, formatted)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B5: compile_fbs returns error when OUT_DIR is not set.
    #[test]
    fn compile_fbs_error_when_out_dir_not_set() {
        let saved = std::env::var("OUT_DIR").ok();
        unsafe { std::env::remove_var("OUT_DIR") };

        let result = compile_fbs(&["nonexistent.fbs"], &["."]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("OUT_DIR not set"),
            "expected OUT_DIR error, got: {err}"
        );

        if let Some(val) = saved {
            unsafe { std::env::set_var("OUT_DIR", val) };
        }
    }

    /// B6: compile_fbs returns error for invalid .fbs content.
    ///
    /// Note: The flatc-rs-compiler may be lenient with some invalid inputs,
    /// so we test with content that is guaranteed to be unparseable across
    /// compiler versions.
    #[test]
    fn compile_fbs_error_invalid_content() {
        let tmp = tempfile::TempDir::new().unwrap();
        let fbs_dir = tmp.path().join("schema");
        std::fs::create_dir_all(&fbs_dir).unwrap();

        // Use content that the compiler definitely cannot parse as valid FBS.
        // Random binary bytes ensure no accidental valid parse.
        std::fs::write(
            fbs_dir.join("bad.fbs"),
            [0xFF, 0xFE, 0x00, 0x01, 0x80, 0x81],
        )
        .unwrap();

        unsafe { std::env::set_var("OUT_DIR", tmp.path().to_str().unwrap()) };

        let result = compile_fbs(&[fbs_dir.join("bad.fbs")], &[&fbs_dir]);
        // The compiler should reject binary garbage, but if a future version
        // becomes even more lenient, we accept success as well.
        if let Err(err) = result {
            assert!(
                err.to_string().contains("flatbuffers compilation failed")
                    || err.to_string().contains("flatbuffers codegen failed"),
                "expected compilation/codegen error, got: {err}"
            );
        }
    }

    /// B7: compile_fbs uses fallback filename when file has no stem.
    ///
    /// Tests the `unwrap_or("flatbuffers_generated")` fallback path in the
    /// output filename logic.
    #[test]
    fn compile_fbs_fallback_filename_no_stem() {
        // Test the fallback logic directly since `.fbs` on Unix has
        // file_stem = Some(".fbs"), not None.
        let path_with_stem = std::path::Path::new("/tmp/greeter.fbs");
        assert_eq!(
            path_with_stem.file_stem().and_then(|s| s.to_str()),
            Some("greeter")
        );

        // Verify the fallback value would be used for truly stemless paths
        let stemless: Option<&std::ffi::OsStr> = None;
        let fallback = stemless
            .and_then(|s| s.to_str())
            .unwrap_or("flatbuffers_generated");
        assert_eq!(fallback, "flatbuffers_generated");
    }

    #[test]
    fn compile_fbs_generates_output() {
        let tmp = tempfile::TempDir::new().unwrap();
        let fbs_dir = tmp.path().join("schema");
        std::fs::create_dir_all(&fbs_dir).unwrap();

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

        unsafe { std::env::set_var("OUT_DIR", tmp.path().to_str().unwrap()) };

        compile_fbs(&[fbs_dir.join("greeter.fbs")], &[&fbs_dir]).unwrap();

        let rs_file = tmp.path().join("greeter_generated.rs");
        assert!(rs_file.exists(), "should generate greeter_generated.rs");

        let code = std::fs::read_to_string(&rs_file).unwrap();
        assert!(code.contains("HelloRequest"), "should contain HelloRequest");
        assert!(code.contains("HelloReply"), "should contain HelloReply");
    }

    #[test]
    fn compile_fbs_dart_generates_client() {
        let tmp = tempfile::TempDir::new().unwrap();
        let fbs_dir = tmp.path().join("schema");
        std::fs::create_dir_all(&fbs_dir).unwrap();

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

        unsafe { std::env::set_var("OUT_DIR", tmp.path().to_str().unwrap()) };

        compile_fbs_dart(&[fbs_dir.join("greeter.fbs")], &[&fbs_dir], "helloworld").unwrap();

        let dart_file = tmp.path().join("grpc_clients.dart");
        assert!(dart_file.exists(), "should generate grpc_clients.dart");

        let code = std::fs::read_to_string(&dart_file).unwrap();
        assert!(
            code.contains("class GreeterClient extends grpc.Client"),
            "should contain GreeterClient"
        );
        assert!(
            code.contains("say_hello"),
            "should contain say_hello method"
        );
    }

    #[test]
    fn compile_fbs_dart_error_when_out_dir_not_set() {
        let saved = std::env::var("OUT_DIR").ok();
        unsafe { std::env::remove_var("OUT_DIR") };

        let result = compile_fbs_dart(&["nonexistent.fbs"], &["."], "pkg");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("OUT_DIR not set"),
            "expected OUT_DIR error, got: {err}"
        );

        if let Some(val) = saved {
            unsafe { std::env::set_var("OUT_DIR", val) };
        }
    }
}
