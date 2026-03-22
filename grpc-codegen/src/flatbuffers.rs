//! Adapter from flatbuffers-rs schema types to grpc-codegen IR.

use crate::ir::{MethodDef, ServiceDef};
use flatc_rs_schema::resolved::{ResolvedRpcCall, ResolvedSchema, ResolvedService};
use flatc_rs_schema::Attributes;
use heck::ToSnakeCase;

/// Default codec path for FlatBuffers-based codegen.
const DEFAULT_CODEC_PATH: &str = "grpc_codec_flatbuffers::FlatBuffersCodec";

/// Convert a flatbuffers-rs `ResolvedService` into a `ServiceDef`.
///
/// `schema` is needed to resolve request/response type names from object indices.
pub fn service_from_fbs(
    service: &ResolvedService,
    schema: &ResolvedSchema,
    proto_path: &str,
) -> Result<ServiceDef, String> {
    let namespace = service
        .namespace
        .as_ref()
        .and_then(|ns| ns.namespace.clone())
        .unwrap_or_default();

    let methods = service
        .calls
        .iter()
        .map(|call| method_from_fbs(call, schema, proto_path))
        .collect::<Result<Vec<_>, _>>()?;

    let name = service.name.clone();
    Ok(ServiceDef {
        proto_name: name.clone(),
        name,
        package: namespace,
        methods,
        comments: extract_comments(&service.documentation),
    })
}

fn method_from_fbs(
    call: &ResolvedRpcCall,
    schema: &ResolvedSchema,
    proto_path: &str,
) -> Result<MethodDef, String> {
    let request_name = &schema
        .objects
        .get(call.request_index)
        .ok_or_else(|| {
            format!(
                "request_index {} out of bounds for schema with {} objects",
                call.request_index,
                schema.objects.len()
            )
        })?
        .name;
    let response_name = &schema
        .objects
        .get(call.response_index)
        .ok_or_else(|| {
            format!(
                "response_index {} out of bounds for schema with {} objects",
                call.response_index,
                schema.objects.len()
            )
        })?
        .name;

    let (client_streaming, server_streaming) = parse_streaming(&call.attributes);

    Ok(MethodDef {
        name: call.name.to_snake_case(),
        proto_name: call.name.clone(),
        input_type: format!("{proto_path}::{request_name}"),
        output_type: format!("{proto_path}::{response_name}"),
        client_streaming,
        server_streaming,
        codec_path: DEFAULT_CODEC_PATH.to_string(),
        comments: extract_comments(&call.documentation),
    })
}

/// Parse streaming mode from FlatBuffers attributes.
///
/// FlatBuffers uses `streaming: "server"`, `streaming: "client"`,
/// or `streaming: "bidi"` key-value attributes on RPC methods.
fn parse_streaming(attrs: &Option<Attributes>) -> (bool, bool) {
    let attrs = match attrs {
        Some(a) => a,
        None => return (false, false),
    };

    for kv in &attrs.entries {
        if kv.key.as_deref() == Some("streaming") {
            return match kv.value.as_deref() {
                Some("server") => (false, true),
                Some("client") => (true, false),
                Some("bidi") => (true, true),
                _ => (false, false),
            };
        }
    }

    (false, false)
}

fn extract_comments(doc: &Option<flatc_rs_schema::Documentation>) -> Vec<String> {
    match doc {
        Some(d) => d.lines.clone(),
        None => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flatc_rs_schema::resolved::{
        ResolvedObject, ResolvedRpcCall, ResolvedSchema, ResolvedService,
    };
    use flatc_rs_schema::{AdvancedFeatures, Attributes, KeyValue, Namespace};

    fn make_schema_with_service() -> (ResolvedSchema, ResolvedService) {
        let objects = vec![
            ResolvedObject {
                name: "HelloRequest".into(),
                fields: vec![],
                is_struct: false,
                min_align: None,
                byte_size: None,
                attributes: None,
                documentation: None,
                declaration_file: None,
                namespace: None,
                span: None,
            },
            ResolvedObject {
                name: "HelloReply".into(),
                fields: vec![],
                is_struct: false,
                min_align: None,
                byte_size: None,
                attributes: None,
                documentation: None,
                declaration_file: None,
                namespace: None,
                span: None,
            },
        ];

        let service = ResolvedService {
            name: "Greeter".into(),
            calls: vec![
                ResolvedRpcCall {
                    name: "SayHello".into(),
                    request_index: 0,
                    response_index: 1,
                    attributes: None,
                    documentation: None,
                    span: None,
                },
                ResolvedRpcCall {
                    name: "SayHelloStream".into(),
                    request_index: 0,
                    response_index: 1,
                    attributes: Some(Attributes {
                        entries: vec![KeyValue {
                            key: Some("streaming".into()),
                            value: Some("server".into()),
                        }],
                    }),
                    documentation: None,
                    span: None,
                },
            ],
            attributes: None,
            documentation: None,
            declaration_file: None,
            namespace: Some(Namespace {
                namespace: Some("helloworld".into()),
            }),
            span: None,
        };

        let schema = ResolvedSchema {
            objects,
            enums: vec![],
            file_ident: None,
            file_ext: None,
            root_table_index: None,
            services: vec![service.clone()],
            advanced_features: AdvancedFeatures::default(),
            fbs_files: vec![],
        };

        (schema, service)
    }

    #[test]
    fn service_from_fbs_basic() {
        let (schema, service) = make_schema_with_service();
        let svc_def = service_from_fbs(&service, &schema, "super").unwrap();

        assert_eq!(svc_def.name, "Greeter");
        assert_eq!(svc_def.package, "helloworld");
        assert_eq!(svc_def.fully_qualified_name(), "helloworld.Greeter");
        assert_eq!(svc_def.methods.len(), 2);

        let m0 = &svc_def.methods[0];
        assert_eq!(m0.name, "say_hello");
        assert_eq!(m0.input_type, "super::HelloRequest");
        assert!(!m0.client_streaming);
        assert!(!m0.server_streaming);

        let m1 = &svc_def.methods[1];
        assert!(m1.server_streaming);
    }

    #[test]
    fn service_from_fbs_invalid_index() {
        let (schema, mut service) = make_schema_with_service();
        service.calls[0].request_index = 999;
        let err = service_from_fbs(&service, &schema, "super").unwrap_err();
        assert!(err.contains("out of bounds"));
    }

    #[test]
    fn parse_streaming_modes() {
        assert_eq!(parse_streaming(&None), (false, false));

        let attrs = |val: &str| {
            Some(Attributes {
                entries: vec![KeyValue {
                    key: Some("streaming".into()),
                    value: Some(val.into()),
                }],
            })
        };

        assert_eq!(parse_streaming(&attrs("server")), (false, true));
        assert_eq!(parse_streaming(&attrs("client")), (true, false));
        assert_eq!(parse_streaming(&attrs("bidi")), (true, true));
        assert_eq!(parse_streaming(&attrs("unknown")), (false, false));
    }
}
