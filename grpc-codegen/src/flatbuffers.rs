//! Adapter from flatbuffers-rs schema types to grpc-codegen IR + FlatBuffers
//! gRPC stubs generator.
//!
//! Two responsibilities live here:
//!
//! 1. **Dart client codegen.** [`generate_dart_client`] emits Dart gRPC client
//!    classes from a [`codegen_schema::SchemaDef`].
//!
//! 2. **Rust gRPC stubs.** [`generate_grpc_stubs`] emits the Rust glue that
//!    bridges the FlatBuffers Object API ([`codegen_flatbuffers`] -> the `*T`
//!    owned types produced by `flatc-rs-codegen` with `gen_object_api: true`)
//!    to `pure-grpc-rs`. Specifically, it produces:
//!
//!    - Top-level type aliases (`pub use generated::<ns>::<Name>T as <Name>;`)
//!      so user code talks about `HelloRequest` rather than `HelloRequestT`.
//!    - `impl FlatBufferGrpcMessage` for every owned wrapper that appears as
//!      an RPC input or output. Encoding goes through `XxxT::pack`; decoding
//!      goes through `flatbuffers::root::<Xxx>(..).unpack()`.
//!    - One `<service>_server` module per `rpc_service` block: the service
//!      trait, an `XxxServer<T>` adapter, per-method `UnaryService` impls,
//!      and a `tower::Service` dispatcher.
//!    - One `<service>_client` module per `rpc_service` block: an
//!      `XxxClient<T>` struct with a `connect(uri)` constructor and one
//!      async method per RPC.
//!
//! The output is intended to be `include!`d at the top level of a crate that
//! also `include!`s the underlying `*_generated.rs` inside a private
//! `mod generated { ... }`. See `examples/greeter-fbs/src/lib.rs` for the
//! canonical layout.

use std::collections::BTreeSet;

use codegen_schema::{MethodDef, SchemaDef, ServiceDef, StreamingType, Type};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::dart_client_gen;
use crate::ir::ServiceDef as IrServiceDef;

/// Codec path used by the FlatBuffers gRPC stubs.
const FB_CODEC_PATH: &str = "grpc_codec_flatbuffers::FlatBuffersCodec";

/// Generate Dart gRPC client code from a flatbuffers-rs `SchemaDef`.
///
/// `schema` should be the result of [`codegen_flatbuffers::from_resolved_schema`].
/// `proto_path` is the import path prefix for the generated Dart types.
///
/// Returns Dart source code for a gRPC client class.
pub fn generate_dart_client(
    schema: &codegen_schema::SchemaDef,
    proto_path: &str,
) -> Result<String, String> {
    // For Dart, we generate client code for all services in the schema
    let mut output = String::new();
    for svc in &schema.services {
        let svc_def: IrServiceDef = svc.clone();
        output.push_str(&dart_client_gen::generate(&svc_def, proto_path));
    }
    Ok(output)
}

/// Generate the FlatBuffers gRPC Rust stubs (owned wrapper aliases,
/// `FlatBufferGrpcMessage` impls, `<svc>_server` modules, `<svc>_client`
/// modules) from a [`codegen_schema::SchemaDef`].
///
/// The returned [`TokenStream`] is intended to be written to a `.rs` file
/// that is `include!`d at the top level of a crate. The `*_generated.rs`
/// file produced by `flatc-rs-codegen` (with `gen_object_api: true`) must
/// be `include!`d into a private module named `generated` -- see the
/// module-level docs.
///
/// Currently only unary RPCs are supported. Streaming variants emit a
/// `compile_error!` in the generated code, since FlatBuffers streaming over
/// gRPC has no production deployment in this workspace yet.
pub fn generate_grpc_stubs(schema: &SchemaDef) -> Result<TokenStream, String> {
    let mut tokens = TokenStream::new();

    // 1. Collect every owned message that appears as an RPC input or output,
    //    plus all messages reachable from those (since the owned `*T` types
    //    inline `*T` for nested table fields and the user constructs them
    //    by name).
    let needed = collect_rpc_relevant_messages(schema);

    // 2. Top-level aliases: `pub use generated::<ns>::<Name>T as <Name>;`
    //    for every needed message. Tables use the `*T` owned variant;
    //    structs are zero-copy in FlatBuffers but their generated `*T`
    //    variants are also owned, so we use the same alias scheme.
    for msg in &schema.messages {
        if !needed.contains(&qualified_name(msg.namespace.as_deref(), &msg.name)) {
            continue;
        }
        let path =
            generated_path_tokens(msg.namespace.as_deref(), &msg.name, true /* owned */);
        let alias = format_ident!("{}", msg.name);
        tokens.extend(quote! {
            pub use #path as #alias;
        });
    }
    // Enums that the user references in RPC input/output types should also
    // be aliased so user code doesn't need to spell out the namespace path.
    for en in &schema.enums {
        // Skip union enums (out of scope; we emit a compile_error! if a
        // union is reached during message walking).
        if en.is_union {
            continue;
        }
        let path = generated_path_tokens(en.namespace.as_deref(), &en.name, false);
        let alias = format_ident!("{}", en.name);
        tokens.extend(quote! {
            pub use #path as #alias;
        });
    }

    // 3. `FlatBufferGrpcMessage` impls -- only for the messages directly
    //    referenced as RPC input or output. Nested tables are encoded
    //    through their parent's `pack`/`unpack`.
    let rpc_io = collect_rpc_io_messages(schema);
    for msg_name in &rpc_io {
        let (ns, name) = split_qualified(msg_name);
        let owned_path = generated_path_tokens(ns.as_deref(), name, true /* owned */);
        let reader_path = generated_path_tokens(ns.as_deref(), name, false /* zero-copy */);
        let alias = format_ident!("{}", name);
        let err_label = format!("invalid {name}: {{e}}");
        tokens.extend(quote! {
            impl grpc_codec_flatbuffers::FlatBufferGrpcMessage for #alias {
                fn encode_flatbuffer(&self) -> ::std::vec::Vec<u8> {
                    let mut builder = grpc_codec_flatbuffers::flatbuffers::FlatBufferBuilder::new();
                    let root = #owned_path::pack(self, &mut builder);
                    builder.finish(root, None);
                    builder.finished_data().to_vec()
                }

                fn decode_flatbuffer(data: &[u8]) -> ::std::result::Result<Self, ::std::string::String> {
                    let reader = grpc_codec_flatbuffers::flatbuffers::root::<#reader_path>(data)
                        .map_err(|e| format!(#err_label))?;
                    Ok(reader.unpack())
                }
            }
        });
    }

    // 4. Per-service server + client modules. Map each FB service to the IR
    //    used by `server_gen` / `client_gen`, with type paths resolved to
    //    the top-level aliases (`super::Xxx`) and the codec path set to
    //    `grpc_codec_flatbuffers::FlatBuffersCodec`.
    for svc in &schema.services {
        let ir = service_to_ir(svc)?;
        tokens.extend(crate::server_gen::generate(&ir));
        tokens.extend(crate::client_gen::generate(&ir));
    }

    Ok(tokens)
}

/// Convert a [`codegen_schema::ServiceDef`] into the IR consumed by
/// `server_gen` / `client_gen`, rewriting type paths so that input/output
/// types resolve through the top-level `Xxx` aliases (i.e. `super::Xxx`)
/// and the codec path resolves to [`FB_CODEC_PATH`].
fn service_to_ir(svc: &ServiceDef) -> Result<crate::ir::ServiceDef, String> {
    let mut methods = Vec::with_capacity(svc.methods.len());
    for m in &svc.methods {
        // Streaming over FlatBuffers is not wired into this stub generator
        // yet -- the `T` <-> reader bridge is unary-only. Bail out loudly
        // so callers don't get half-generated streaming endpoints.
        if !matches!(m.streaming, StreamingType::None) {
            return Err(format!(
                "FlatBuffers gRPC stub generator does not yet support streaming RPC `{}` (mode: {})",
                m.name,
                m.streaming_mode()
            ));
        }
        methods.push(MethodDef {
            name: m.name.clone(),
            rust_name: m.rust_name.clone(),
            input_type: format!("super::{}", m.input_type),
            output_type: format!("super::{}", m.output_type),
            streaming: m.streaming,
            codec_path: FB_CODEC_PATH.to_string(),
            comments: m.comments.clone(),
        });
    }
    Ok(crate::ir::ServiceDef {
        name: svc.name.clone(),
        package: svc.package.clone(),
        methods,
        comments: svc.comments.clone(),
    })
}

/// Build a `proc_macro2` token stream for the path of a generated type.
///
/// `owned = true` produces the `*T` owned variant; `owned = false` produces
/// the zero-copy reader name. The path is prefixed with `generated::`,
/// mirroring the `mod generated { include!(...) }` layout that callers are
/// expected to use.
fn generated_path_tokens(namespace: Option<&str>, name: &str, owned: bool) -> TokenStream {
    let mut segments: Vec<TokenStream> = vec![quote! { generated }];
    if let Some(ns) = namespace.filter(|n| !n.is_empty()) {
        for part in ns.split('.') {
            // FlatBuffers namespaces become snake_case Rust modules.
            let mod_name = format_ident!("{}", to_snake_case(part));
            segments.push(quote! { #mod_name });
        }
    }
    let type_name = if owned {
        format_ident!("{}T", name)
    } else {
        format_ident!("{}", name)
    };
    segments.push(quote! { #type_name });
    quote! { #(#segments)::* }
}

/// Convert a FlatBuffers namespace component to snake_case the same way
/// `flatc-rs-codegen` does (matches `type_map::to_snake_case`).
fn to_snake_case(s: &str) -> String {
    // `flatc-rs-codegen` lowercases the namespace component verbatim for
    // single-word components ("v1" stays "v1") and inserts underscores
    // between camel-case boundaries for multi-word ones ("MyGame" ->
    // "my_game"). We use heck for parity.
    use heck::ToSnakeCase;
    s.to_snake_case()
}

/// Build a stable identifier for a message: `"<namespace>.<name>"` if
/// the namespace is set, otherwise just `<name>`.
fn qualified_name(namespace: Option<&str>, name: &str) -> String {
    match namespace.filter(|n| !n.is_empty()) {
        Some(ns) => format!("{ns}.{name}"),
        None => name.to_string(),
    }
}

/// Split a `"<namespace>.<name>"` identifier produced by [`qualified_name`]
/// back into its namespace + name components.
fn split_qualified(id: &str) -> (Option<String>, &str) {
    match id.rfind('.') {
        Some(idx) => (Some(id[..idx].to_string()), &id[idx + 1..]),
        None => (None, id),
    }
}

/// Walk every RPC and return the set of messages used directly as input or
/// output. The names are namespace-qualified via [`qualified_name`].
fn collect_rpc_io_messages(schema: &SchemaDef) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    // Build a name -> namespace lookup for messages so we can promote bare
    // type names from MethodDef into namespace-qualified ids.
    for svc in &schema.services {
        for method in &svc.methods {
            if let Some(qn) = qualify_message(schema, &method.input_type) {
                out.insert(qn);
            }
            if let Some(qn) = qualify_message(schema, &method.output_type) {
                out.insert(qn);
            }
        }
    }
    out
}

/// Walk every RPC input/output and transitively walk every nested message
/// reachable through their fields (and through vector / optional element
/// types). Returns the set of namespace-qualified message names.
///
/// We use this to decide which messages get top-level aliases. Aliasing a
/// nested type lets users write `Task { .. }` instead of `taskq::v1::TaskT { .. }`
/// when constructing RPC payloads.
fn collect_rpc_relevant_messages(schema: &SchemaDef) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut frontier: Vec<String> = collect_rpc_io_messages(schema).into_iter().collect();

    while let Some(qn) = frontier.pop() {
        if !out.insert(qn.clone()) {
            continue;
        }
        let (ns, name) = split_qualified(&qn);
        let msg = match find_message(schema, ns.as_deref(), name) {
            Some(m) => m,
            None => continue,
        };
        for field in &msg.fields {
            collect_referenced_messages(&field.ty, schema, &mut frontier);
        }
    }
    out
}

/// Walk a [`Type`] and push every transitively-referenced message id onto
/// `out_frontier`.
fn collect_referenced_messages(ty: &Type, schema: &SchemaDef, out_frontier: &mut Vec<String>) {
    match ty {
        Type::Scalar(_) | Type::Enum { .. } | Type::ForeignKey(_) => {}
        Type::Message { name, package } => {
            // Resolve to the canonical namespace via the schema's message
            // list -- the IR's `package` may be empty when the message lives
            // in the root namespace.
            if let Some(msg) = find_message(schema, package.as_deref(), name) {
                out_frontier.push(qualified_name(msg.namespace.as_deref(), &msg.name));
            }
        }
        Type::Vector(inner) | Type::Optional(inner) => {
            collect_referenced_messages(inner, schema, out_frontier);
        }
        Type::Map { key, value } => {
            collect_referenced_messages(key, schema, out_frontier);
            collect_referenced_messages(value, schema, out_frontier);
        }
        Type::OneOf { variants, .. } => {
            for v in variants {
                collect_referenced_messages(&v.ty, schema, out_frontier);
            }
        }
    }
}

/// Look up a message by name within the schema. The lookup tolerates a
/// missing namespace -- [`MethodDef::input_type`] from the FlatBuffers
/// adapter is the bare table name with no namespace prefix.
fn find_message<'a>(
    schema: &'a SchemaDef,
    namespace: Option<&str>,
    name: &str,
) -> Option<&'a codegen_schema::MessageDef> {
    schema.messages.iter().find(|m| {
        m.name == name
            && match (namespace, m.namespace.as_deref()) {
                (None, _) => true,
                (Some(""), _) => true,
                (Some(req), Some(have)) => req == have,
                (Some(_), None) => false,
            }
    })
}

/// Given an `input_type` / `output_type` string from a FlatBuffers
/// `MethodDef`, return the namespace-qualified message name used in our
/// internal sets, or `None` if no matching message exists.
///
/// FlatBuffers adapter writes bare names (`SubmitTaskRequest`), so we look
/// them up against the schema's message list.
fn qualify_message(schema: &SchemaDef, raw: &str) -> Option<String> {
    // The IR may carry a `super::` prefix or other path noise from earlier
    // codegen stages; strip everything before the last `::` and call that
    // the bare name.
    let bare = raw.rsplit("::").next().unwrap_or(raw);
    schema
        .messages
        .iter()
        .find(|m| m.name == bare)
        .map(|m| qualified_name(m.namespace.as_deref(), &m.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegen_schema::{FieldDef, MessageDef, ScalarType};

    fn greeter_schema() -> SchemaDef {
        SchemaDef {
            name: "greeter".into(),
            file_ident: None,
            root_table: None,
            messages: vec![
                MessageDef {
                    name: "HelloRequest".into(),
                    namespace: Some("helloworld".into()),
                    is_struct: false,
                    comments: vec![],
                    fields: vec![FieldDef {
                        name: "name".into(),
                        ty: Type::Scalar(ScalarType::String),
                        is_optional: true,
                        default_value: None,
                        id: Some(0),
                        comments: vec![],
                    }],
                },
                MessageDef {
                    name: "HelloReply".into(),
                    namespace: Some("helloworld".into()),
                    is_struct: false,
                    comments: vec![],
                    fields: vec![FieldDef {
                        name: "message".into(),
                        ty: Type::Scalar(ScalarType::String),
                        is_optional: true,
                        default_value: None,
                        id: Some(0),
                        comments: vec![],
                    }],
                },
            ],
            enums: vec![],
            services: vec![ServiceDef {
                name: "Greeter".into(),
                package: Some("helloworld".into()),
                comments: vec![],
                methods: vec![MethodDef {
                    name: "SayHello".into(),
                    rust_name: None,
                    input_type: "HelloRequest".into(),
                    output_type: "HelloReply".into(),
                    streaming: StreamingType::None,
                    codec_path: "crate::codec::Codec".into(),
                    comments: vec![],
                }],
            }],
        }
    }

    #[test]
    fn stubs_compile_as_a_token_stream() {
        // Arrange
        let schema = greeter_schema();

        // Act
        let tokens = generate_grpc_stubs(&schema).expect("schema must produce stubs");

        // Assert: the output parses as a Rust file. Anything that doesn't
        // would be a syntactic regression in the generator.
        let parsed = syn::parse2::<syn::File>(quote! { #tokens });
        assert!(
            parsed.is_ok(),
            "generated grpc stubs should be syntactically valid: {:?}",
            parsed.err()
        );
    }

    #[test]
    fn stubs_alias_owned_wrappers_at_top_level() {
        // Arrange
        let schema = greeter_schema();

        // Act
        let code = generate_grpc_stubs(&schema).unwrap().to_string();

        // Assert: the user-facing alias `HelloRequest` -> `HelloRequestT`.
        assert!(
            code.contains("HelloRequestT as HelloRequest"),
            "missing owned wrapper alias for HelloRequest: {code}"
        );
        assert!(
            code.contains("HelloReplyT as HelloReply"),
            "missing owned wrapper alias for HelloReply: {code}"
        );
    }

    #[test]
    fn stubs_emit_codec_impl_for_each_rpc_message() {
        // Arrange
        let schema = greeter_schema();

        // Act
        let code = generate_grpc_stubs(&schema).unwrap().to_string();

        // Assert
        assert!(
            code.contains("FlatBufferGrpcMessage for HelloRequest"),
            "missing codec impl for HelloRequest: {code}"
        );
        assert!(
            code.contains("FlatBufferGrpcMessage for HelloReply"),
            "missing codec impl for HelloReply: {code}"
        );
    }

    #[test]
    fn stubs_emit_server_and_client_modules() {
        // Arrange
        let schema = greeter_schema();

        // Act
        let code = generate_grpc_stubs(&schema).unwrap().to_string();

        // Assert
        assert!(
            code.contains("pub mod greeter_server"),
            "missing greeter_server module: {code}"
        );
        assert!(
            code.contains("pub mod greeter_client"),
            "missing greeter_client module: {code}"
        );
        assert!(
            code.contains("trait Greeter"),
            "server module should declare the service trait: {code}"
        );
        assert!(
            code.contains("GreeterClient"),
            "client module should declare the client struct: {code}"
        );
    }

    #[test]
    fn streaming_methods_yield_an_error() {
        // Arrange: same schema but flip the RPC to server-streaming.
        let mut schema = greeter_schema();
        schema.services[0].methods[0].streaming = StreamingType::Server;

        // Act
        let result = generate_grpc_stubs(&schema);

        // Assert
        assert!(
            result.is_err(),
            "streaming should be rejected by the FB stub generator (until streaming bridge lands)"
        );
    }
}
