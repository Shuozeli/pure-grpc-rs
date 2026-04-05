//! Adapter from flatbuffers-rs schema types to grpc-codegen IR.
//!
//! This module provides Dart client generation from a [`codegen_schema::SchemaDef`].
//! For Rust server/client generation, use the generators directly with [`crate::ir::ServiceDef`].

use crate::dart_client_gen;
use crate::ir::ServiceDef;

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
        let svc_def: ServiceDef = svc.clone();
        output.push_str(&dart_client_gen::generate(&svc_def, proto_path));
    }
    Ok(output)
}

/// Generate gRPC service code (server + client) from a [`codegen_schema::SchemaDef`].
///
/// This is a convenience function that converts the schema and generates
/// server and client code for all services.
pub fn generate_service_tokens(
    schema: &codegen_schema::SchemaDef,
) -> Result<(proc_macro2::TokenStream, proc_macro2::TokenStream), String> {
    let mut server_tokens = proc_macro2::TokenStream::new();
    let mut client_tokens = proc_macro2::TokenStream::new();

    for svc in &schema.services {
        let svc_def: ServiceDef = svc.clone();
        server_tokens.extend(crate::server_gen::generate(&svc_def));
        client_tokens.extend(crate::client_gen::generate(&svc_def));
    }

    Ok((server_tokens, client_tokens))
}
