//! Compile .proto files into Rust gRPC service stubs.

use grpc_codegen::protobuf::service_from_proto;
use grpc_codegen::{client_gen, server_gen};
use proc_macro2::TokenStream;
use std::io;
use std::path::Path;

/// Compile `.proto` files into Rust message types + gRPC service stubs.
///
/// Parses and analyzes the proto files using protobuf-rs, generates message
/// types via protoc-rs-codegen, then generates gRPC server/client stubs
/// via grpc-codegen. All output is written to `OUT_DIR`.
///
/// # Arguments
///
/// * `protos` - Paths to `.proto` files to compile.
/// * `includes` - Include directories for import resolution.
///
/// # Example
///
/// In `build.rs`:
/// ```ignore
/// fn main() {
///     grpc_build::compile_protos(
///         &["proto/helloworld.proto"],
///         &["proto"],
///     ).unwrap();
/// }
/// ```
pub fn compile_protos(
    protos: &[impl AsRef<Path>],
    includes: &[impl AsRef<Path>],
) -> io::Result<()> {
    let out_dir =
        std::env::var("OUT_DIR").map_err(|e| io::Error::other(format!("OUT_DIR not set: {e}")))?;

    // Build a file resolver from include paths
    let resolver = DirResolver::new(includes);

    for proto_path in protos {
        let proto_path = proto_path.as_ref();
        let _source = std::fs::read_to_string(proto_path).map_err(|e| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("failed to read {}: {e}", proto_path.display()),
            )
        })?;

        let file_name = proto_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown.proto");

        // Step 1: Analyze (parse + type resolution)
        let root_files = &[file_name];
        let fds = protoc_rs_analyzer::analyze_files(root_files, &resolver).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to analyze {}: {e:?}", proto_path.display()),
            )
        })?;

        // Step 2: Generate message types (structs/enums)
        let message_code = protoc_rs_codegen::generate_rust(&fds).map_err(|e| {
            io::Error::other(format!(
                "codegen failed for {}: {e:?}",
                proto_path.display()
            ))
        })?;

        // Step 3: Generate gRPC service stubs
        let mut service_tokens = TokenStream::new();
        for file in &fds.file {
            let package = file.package.as_deref().unwrap_or("");
            for svc_proto in &file.service {
                let svc_def = service_from_proto(svc_proto, package, "super");
                service_tokens.extend(server_gen::generate(&svc_def));
                service_tokens.extend(client_gen::generate(&svc_def));
            }
        }

        // Step 4: Write output
        let service_code = if service_tokens.is_empty() {
            String::new()
        } else {
            let file = syn::parse2::<syn::File>(service_tokens)
                .map_err(|e| io::Error::other(format!("generated service code is invalid: {e}")))?;
            prettyplease::unparse(&file)
        };

        // Generate FILE_DESCRIPTOR_SET const for reflection
        let descriptor_const = "\n/// Encoded `FileDescriptorSet` for gRPC server reflection.\n\
            /// Pass to `ReflectionServer::builder().register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)`.\n\
            pub const FILE_DESCRIPTOR_SET: &[u8] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/descriptor.bin\"));\n";

        // Write message code + service code + descriptor const to OUT_DIR
        for (mod_path, code) in &message_code {
            let out_file = Path::new(&out_dir).join(mod_path);
            let combined = format!("{code}\n{service_code}\n{descriptor_const}");
            std::fs::write(&out_file, combined)?;
        }

        // Step 5: Write descriptor set for reflection
        let descriptor_bytes = protoc_rs_compiler::descriptor_set::serialize_descriptor_set(&fds);
        let descriptor_file = Path::new(&out_dir).join("descriptor.bin");
        std::fs::write(&descriptor_file, descriptor_bytes)?;

        // If no message code was generated but we have services, write just services
        if message_code.is_empty() && !service_code.is_empty() {
            let mod_name = package_to_mod_name(
                &fds.file
                    .first()
                    .and_then(|f| f.package.clone())
                    .unwrap_or_default(),
            );
            let out_file = Path::new(&out_dir).join(format!("{mod_name}.rs"));
            std::fs::write(&out_file, service_code)?;
        }
    }

    Ok(())
}

fn package_to_mod_name(package: &str) -> String {
    if package.is_empty() {
        "_.rs".to_string()
    } else {
        package.replace('.', "_")
    }
}

/// A simple directory-based file resolver for proto imports.
struct DirResolver {
    dirs: Vec<std::path::PathBuf>,
}

impl DirResolver {
    fn new(dirs: &[impl AsRef<Path>]) -> Self {
        Self {
            dirs: dirs.iter().map(|d| d.as_ref().to_path_buf()).collect(),
        }
    }
}

impl protoc_rs_analyzer::FileResolver for DirResolver {
    fn resolve(&self, name: &str) -> Option<String> {
        for dir in &self.dirs {
            let path = dir.join(name);
            if let Ok(content) = std::fs::read_to_string(&path) {
                return Some(content);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_to_mod_name_converts_dots() {
        assert_eq!(package_to_mod_name("helloworld"), "helloworld");
        assert_eq!(package_to_mod_name("google.protobuf"), "google_protobuf");
        assert_eq!(package_to_mod_name(""), "_.rs");
    }

    #[test]
    fn compile_protos_generates_output() {
        let tmp = tempfile::TempDir::new().unwrap();
        let proto_dir = tmp.path().join("proto");
        std::fs::create_dir_all(&proto_dir).unwrap();

        std::fs::write(
            proto_dir.join("test.proto"),
            r#"syntax = "proto3";
package testpkg;
message Ping { string msg = 1; }
message Pong { string msg = 1; }
service Echo { rpc Send(Ping) returns (Pong) {} }
"#,
        )
        .unwrap();

        // Set OUT_DIR for the compile
        std::env::set_var("OUT_DIR", tmp.path().to_str().unwrap());

        compile_protos(&[proto_dir.join("test.proto")], &[&proto_dir]).unwrap();

        // Should produce a .rs file
        let rs_file = tmp.path().join("testpkg.rs");
        assert!(rs_file.exists(), "should generate testpkg.rs");

        let code = std::fs::read_to_string(&rs_file).unwrap();
        assert!(code.contains("Ping"), "should contain Ping message");
        assert!(code.contains("Pong"), "should contain Pong message");
        assert!(code.contains("echo_server"), "should contain server module");
        assert!(code.contains("echo_client"), "should contain client module");
        assert!(
            code.contains("FILE_DESCRIPTOR_SET"),
            "should contain descriptor const"
        );

        // Should produce descriptor.bin
        let descriptor = tmp.path().join("descriptor.bin");
        assert!(descriptor.exists(), "should generate descriptor.bin");
        assert!(
            std::fs::read(&descriptor).unwrap().len() > 10,
            "descriptor should be non-trivial"
        );
    }
}
