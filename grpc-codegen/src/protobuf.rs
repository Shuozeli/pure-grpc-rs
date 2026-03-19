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
pub fn service_from_proto(
    proto: &ServiceDescriptorProto,
    package: &str,
    proto_path: &str,
) -> ServiceDef {
    let name = proto
        .name
        .as_deref()
        .filter(|s| !s.is_empty())
        .expect("service must have a name")
        .to_string();
    let methods = proto
        .method
        .iter()
        .map(|m| method_from_proto(m, proto_path))
        .collect();

    ServiceDef {
        name: name.clone(),
        package: package.to_string(),
        proto_name: name,
        methods,
        comments: vec![],
    }
}

fn method_from_proto(proto: &MethodDescriptorProto, proto_path: &str) -> MethodDef {
    let proto_name = proto
        .name
        .as_deref()
        .filter(|s| !s.is_empty())
        .expect("method must have a name")
        .to_string();
    let name = proto_name.to_snake_case();

    let input_type = resolve_type(
        proto
            .input_type
            .as_deref()
            .filter(|s| !s.is_empty())
            .expect("method must have an input_type"),
        proto_path,
    );
    let output_type = resolve_type(
        proto
            .output_type
            .as_deref()
            .filter(|s| !s.is_empty())
            .expect("method must have an output_type"),
        proto_path,
    );

    MethodDef {
        name,
        proto_name,
        input_type,
        output_type,
        client_streaming: proto.client_streaming.unwrap_or(false),
        server_streaming: proto.server_streaming.unwrap_or(false),
        codec_path: DEFAULT_CODEC_PATH.to_string(),
        comments: vec![],
    }
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

        let svc = service_from_proto(&proto, "helloworld", "super");

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
}
