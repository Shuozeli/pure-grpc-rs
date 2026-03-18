//! Hand-written prost message types for grpc.reflection.v1.
//!
//! Matches the official [reflection.proto](https://github.com/grpc/grpc-proto/blob/master/grpc/reflection/v1/reflection.proto).

#[derive(Clone, PartialEq, prost::Message)]
pub struct ServerReflectionRequest {
    #[prost(string, tag = "1")]
    pub host: String,
    #[prost(
        oneof = "server_reflection_request::MessageRequest",
        tags = "3, 4, 5, 6, 7"
    )]
    pub message_request: Option<server_reflection_request::MessageRequest>,
}

pub mod server_reflection_request {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum MessageRequest {
        #[prost(string, tag = "3")]
        FileByFilename(String),
        #[prost(string, tag = "4")]
        FileContainingSymbol(String),
        #[prost(message, tag = "5")]
        FileContainingExtension(super::ExtensionRequest),
        #[prost(string, tag = "6")]
        AllExtensionNumbersOfType(String),
        #[prost(string, tag = "7")]
        ListServices(String),
    }
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ExtensionRequest {
    #[prost(string, tag = "1")]
    pub containing_type: String,
    #[prost(int32, tag = "2")]
    pub extension_number: i32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ServerReflectionResponse {
    #[prost(string, tag = "1")]
    pub valid_host: String,
    #[prost(message, optional, tag = "2")]
    pub original_request: Option<ServerReflectionRequest>,
    #[prost(
        oneof = "server_reflection_response::MessageResponse",
        tags = "4, 5, 6, 7"
    )]
    pub message_response: Option<server_reflection_response::MessageResponse>,
}

pub mod server_reflection_response {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum MessageResponse {
        #[prost(message, tag = "4")]
        FileDescriptorResponse(super::FileDescriptorResponse),
        #[prost(message, tag = "5")]
        AllExtensionNumbersResponse(super::ExtensionNumberResponse),
        #[prost(message, tag = "6")]
        ListServicesResponse(super::ListServiceResponse),
        #[prost(message, tag = "7")]
        ErrorResponse(super::ErrorResponse),
    }
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct FileDescriptorResponse {
    #[prost(bytes = "vec", repeated, tag = "1")]
    pub file_descriptor_proto: Vec<Vec<u8>>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ExtensionNumberResponse {
    #[prost(string, tag = "1")]
    pub base_type_name: String,
    #[prost(int32, repeated, tag = "2")]
    pub extension_number: Vec<i32>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ListServiceResponse {
    #[prost(message, repeated, tag = "1")]
    pub service: Vec<ServiceResponse>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ServiceResponse {
    #[prost(string, tag = "1")]
    pub name: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ErrorResponse {
    #[prost(int32, tag = "1")]
    pub error_code: i32,
    #[prost(string, tag = "2")]
    pub error_message: String,
}
