//! Compile .fbs files into Rust FlatBuffers types + gRPC service stubs.

use std::io;
use std::path::{Path, PathBuf};

/// Compile `.fbs` files into Rust FlatBuffers types + gRPC service stubs.
///
/// Parses and analyzes the FlatBuffers schema using flatbuffers-rs, generates
/// Rust readers/builders via flatc-rs-codegen (with gRPC stubs if services
/// are defined). All output is written to `OUT_DIR`.
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

    let options = flatc_rs_compiler::CompilerOptions {
        include_paths: include_paths.clone(),
    };

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

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

        std::env::set_var("OUT_DIR", tmp.path().to_str().unwrap());

        compile_fbs(&[fbs_dir.join("greeter.fbs")], &[&fbs_dir]).unwrap();

        let rs_file = tmp.path().join("greeter_generated.rs");
        assert!(rs_file.exists(), "should generate greeter_generated.rs");

        let code = std::fs::read_to_string(&rs_file).unwrap();
        assert!(code.contains("HelloRequest"), "should contain HelloRequest");
        assert!(code.contains("HelloReply"), "should contain HelloReply");
    }
}
