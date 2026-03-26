//! Adapter from protobuf-rs schema types to grpc-codegen IR.

use crate::ir::{MethodDef, ServiceDef};
use heck::ToSnakeCase;
use protoc_rs_schema::{MethodDescriptorProto, ServiceDescriptorProto};

/// Default codec path for prost-based codegen.
const DEFAULT_CODEC_PATH: &str = "grpc_core::codec::prost_codec::ProstCodec";

/// Convert a protobuf-rs `ServiceDescriptorProto` into a `ServiceDef`.
///
/// `package` is the proto package name (e.g., `"helloworld"`).
/// `proto_path` is the Rust module prefix for types (e.g., `"super"` or `"crate"`).
///
/// Returns an error if the service or any of its methods are missing required fields.
///
/// Note: `comments` are not populated because proto descriptors carry documentation
/// in `SourceCodeInfo` (on `FileDescriptorProto`), not on individual descriptors.
/// Wiring up source comments requires passing the file-level `SourceCodeInfo` and
/// computing source paths -- left for a future codegen enhancement.
pub fn service_from_proto(
    proto: &ServiceDescriptorProto,
    package: &str,
    proto_path: &str,
) -> Result<ServiceDef, String> {
    let name = proto
        .name
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or("service descriptor is missing a name")?
        .to_string();
    let methods = proto
        .method
        .iter()
        .map(|m| method_from_proto(m, proto_path))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(ServiceDef {
        name: name.clone(),
        package: package.to_string(),
        proto_name: name,
        methods,
        comments: vec![],
    })
}

fn method_from_proto(proto: &MethodDescriptorProto, proto_path: &str) -> Result<MethodDef, String> {
    let proto_name = proto
        .name
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or("method descriptor is missing a name")?
        .to_string();
    let name = proto_name.to_snake_case();

    let input_type = resolve_type(
        proto
            .input_type
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("method `{proto_name}` is missing an input_type"))?,
        proto_path,
    );
    let output_type = resolve_type(
        proto
            .output_type
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("method `{proto_name}` is missing an output_type"))?,
        proto_path,
    );

    Ok(MethodDef {
        name,
        proto_name,
        input_type,
        output_type,
        client_streaming: proto.client_streaming.unwrap_or(false),
        server_streaming: proto.server_streaming.unwrap_or(false),
        codec_path: DEFAULT_CODEC_PATH.to_string(),
        comments: vec![],
    })
}

/// Resolve a proto type reference (e.g., `.helloworld.HelloRequest`) into a Rust type path.
///
/// Since messages and services are generated into the same file, service modules
/// use `super::` to reach the message types. We only need `{proto_path}::{TypeName}`,
/// stripping the package segments since they correspond to the file itself.
fn resolve_type(proto_type: &str, proto_path: &str) -> String {
    let cleaned = proto_type.strip_prefix('.').unwrap_or(proto_type);

    // The last segment is the type name, everything before is the package
    let type_name = cleaned.rsplit('.').next().unwrap_or(cleaned);

    format!("{proto_path}::{type_name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_type_simple() {
        assert_eq!(
            resolve_type(".helloworld.HelloRequest", "super"),
            "super::HelloRequest"
        );
    }

    #[test]
    fn resolve_type_nested_package() {
        assert_eq!(
            resolve_type(".google.protobuf.Empty", "super"),
            "super::Empty"
        );
    }

    #[test]
    fn resolve_type_no_leading_dot() {
        assert_eq!(resolve_type("HelloRequest", "super"), "super::HelloRequest");
    }

    #[test]
    fn service_from_proto_basic() {
        let proto = ServiceDescriptorProto {
            name: Some("Greeter".into()),
            method: vec![
                MethodDescriptorProto {
                    name: Some("SayHello".into()),
                    input_type: Some(".helloworld.HelloRequest".into()),
                    output_type: Some(".helloworld.HelloReply".into()),
                    options: None,
                    client_streaming: Some(false),
                    server_streaming: Some(false),
                },
                MethodDescriptorProto {
                    name: Some("SayHelloStream".into()),
                    input_type: Some(".helloworld.HelloRequest".into()),
                    output_type: Some(".helloworld.HelloReply".into()),
                    options: None,
                    client_streaming: Some(false),
                    server_streaming: Some(true),
                },
            ],
            options: None,
        };

        let svc = service_from_proto(&proto, "helloworld", "super").unwrap();

        assert_eq!(svc.name, "Greeter");
        assert_eq!(svc.package, "helloworld");
        assert_eq!(svc.fully_qualified_name(), "helloworld.Greeter");
        assert_eq!(svc.methods.len(), 2);

        let m0 = &svc.methods[0];
        assert_eq!(m0.name, "say_hello");
        assert_eq!(m0.proto_name, "SayHello");
        assert_eq!(m0.input_type, "super::HelloRequest");
        assert!(!m0.client_streaming);
        assert!(!m0.server_streaming);

        let m1 = &svc.methods[1];
        assert_eq!(m1.name, "say_hello_stream");
        assert!(m1.server_streaming);
    }

    #[test]
    fn service_from_proto_missing_name_is_err() {
        let proto = ServiceDescriptorProto {
            name: None,
            method: vec![],
            options: None,
        };
        let result = service_from_proto(&proto, "pkg", "super");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing a name"));
    }

    #[test]
    fn method_missing_input_type_is_err() {
        let proto = ServiceDescriptorProto {
            name: Some("Svc".into()),
            method: vec![MethodDescriptorProto {
                name: Some("Rpc".into()),
                input_type: None,
                output_type: Some(".pkg.Out".into()),
                options: None,
                client_streaming: None,
                server_streaming: None,
            }],
            options: None,
        };
        let result = service_from_proto(&proto, "pkg", "super");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("input_type"));
    }
}
