//! gRPC Todo Service Load Test Binary
//!
//! Compares gRPC performance across three server implementations:
//! - Tonic (port 50053) - using tonic with prost codec
//! - pure-grpc prost (port 50052) - using pure-grpc-rs with prost codec
//! - pure-grpc FlatBuffers (port 50051) - using pure-grpc-rs with FlatBuffers codec
//!
//! Run with: `cargo run --release -p benchmarks --bin todo_load_test -- [options]`
//!
//! Options:
//!   --duration <secs>          Test duration in seconds (default: 10)
//!   --concurrency <n>          Number of concurrent workers (default: 50)
//!   --flatbuffers-port <port>  Port for FlatBuffers server (default: 50051)
//!   --prost-port <port>        Port for Prost server (default: 50052)
//!   --tonic-port <port>        Port for Tonic server (default: 50053)

#![allow(clippy::all, unused, dead_code)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;
use http::uri::PathAndQuery;
use tokio::sync::{mpsc, Barrier};
use tokio::time::timeout;

use grpc_client::{Channel, Grpc};
use grpc_codec_flatbuffers::{FlatBufferGrpcMessage, FlatBuffersCodec};
use grpc_core::codec::prost_codec::ProstCodec;
use grpc_core::extensions::GrpcMethod;
use grpc_core::request::IntoRequest;
use grpc_core::{Response, Status};

// ============================================================================
// Manual Prost Types (for tonic client - avoids conflict with grpc_build)
// ============================================================================
use prost::Message;

#[derive(Clone, Message)]
pub struct Todo {
    #[prost(string, tag = "1")]
    pub id: String,
    #[prost(string, tag = "2")]
    pub title: String,
    #[prost(string, tag = "3")]
    pub description: String,
    #[prost(bool, tag = "4")]
    pub completed: bool,
    #[prost(int64, tag = "5")]
    pub created_at: i64,
}

#[derive(Clone, Message)]
pub struct CreateTodoRequest {
    #[prost(string, tag = "1")]
    pub title: String,
    #[prost(string, tag = "2")]
    pub description: String,
}

#[derive(Clone, Message)]
pub struct CreateTodoResponse {
    #[prost(message, optional, tag = "1")]
    pub todo: Option<Todo>,
}

#[derive(Clone, Message)]
pub struct GetTodoRequest {
    #[prost(string, tag = "1")]
    pub id: String,
}

#[derive(Clone, Message)]
pub struct GetTodoResponse {
    #[prost(message, optional, tag = "1")]
    pub todo: Option<Todo>,
}

#[derive(Clone, Message)]
pub struct ListTodosRequest {}

#[derive(Clone, Message)]
pub struct ListTodosResponse {
    #[prost(message, repeated, tag = "1")]
    pub todos: Vec<Todo>,
}

#[derive(Clone, Message)]
pub struct DeleteTodoRequest {
    #[prost(string, tag = "1")]
    pub id: String,
}

#[derive(Clone, Message)]
pub struct DeleteTodoResponse {}

// ============================================================================
// Prost Generated Code (from grpc_build)
// ============================================================================
#[allow(clippy::all)]
#[allow(non_snake_case)]
mod generated_prost {
    include!(concat!(env!("OUT_DIR"), "/todo.rs"));
}

use generated_prost::{
    CreateTodoRequest as ProstCreateTodoRequest, CreateTodoResponse as ProstCreateTodoResponse,
    DeleteTodoRequest as ProstDeleteTodoRequest, DeleteTodoResponse as ProstDeleteTodoResponse,
    GetTodoRequest as ProstGetTodoRequest, GetTodoResponse as ProstGetTodoResponse,
    ListTodosRequest as ProstListTodosRequest, ListTodosResponse as ProstListTodosResponse,
};

// ============================================================================
// FlatBuffers Generated Code (from grpc_build with flatbuffers feature)
// ============================================================================
#[allow(
    unused_imports,
    dead_code,
    non_snake_case,
    clippy::extra_unused_lifetimes,
    clippy::needless_lifetimes,
    clippy::derivable_impls,
    clippy::unnecessary_cast
)]
mod generated_flatbuffers {
    include!(concat!(env!("OUT_DIR"), "/todo_generated.rs"));
}

// Import FlatBuffers table types from generated code
use generated_flatbuffers::todo::{
    Todo as FbsTodo, TodoArgs as FbsTodoArgs,
    CreateTodoRequest as FbsCreateTodoRequest, CreateTodoRequestArgs as FbsCreateTodoRequestArgs,
    CreateTodoResponse as FbsCreateTodoResponse, CreateTodoResponseArgs as FbsCreateTodoResponseArgs,
    GetTodoRequest as FbsGetTodoRequest, GetTodoRequestArgs as FbsGetTodoRequestArgs,
    GetTodoResponse as FbsGetTodoResponse, GetTodoResponseArgs as FbsGetTodoResponseArgs,
    ListTodosRequest as FbsListTodosRequest, ListTodosRequestArgs as FbsListTodosRequestArgs,
    ListTodosResponse as FbsListTodosResponse, ListTodosResponseArgs as FbsListTodosResponseArgs,
    DeleteTodoRequest as FbsDeleteTodoRequest, DeleteTodoRequestArgs as FbsDeleteTodoRequestArgs,
    DeleteTodoResponse as FbsDeleteTodoResponse, DeleteTodoResponseArgs as FbsDeleteTodoResponseArgs,
};

// ============================================================================
// FlatBuffers Owned Types (implement FlatBufferGrpcMessage)
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct TodoT {
    pub id: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub completed: bool,
    pub created_at: i64,
}

impl Default for TodoT {
    fn default() -> Self {
        Self {
            id: None,
            title: None,
            description: None,
            completed: false,
            created_at: 0,
        }
    }
}

impl FlatBufferGrpcMessage for TodoT {
    fn encode_flatbuffer(&self) -> Vec<u8> {
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let id = self.id.as_ref().map(|x| builder.create_string(x));
        let title = self.title.as_ref().map(|x| builder.create_string(x));
        let description = self.description.as_ref().map(|x| builder.create_string(x));
        let todo = FbsTodo::create(
            &mut builder,
            &FbsTodoArgs {
                id,
                title,
                description,
                completed: self.completed,
                created_at: self.created_at,
            },
        );
        builder.finish(todo, None);
        builder.finished_data().to_vec()
    }

    fn decode_flatbuffer(data: &[u8]) -> Result<Self, String> {
        let req = flatbuffers::root::<FbsTodo>(data)
            .map_err(|e| format!("invalid Todo: {e}"))?;
        Ok(TodoT {
            id: req.id().map(|x| x.to_string()),
            title: req.title().map(|x| x.to_string()),
            description: req.description().map(|x| x.to_string()),
            completed: req.completed(),
            created_at: req.created_at(),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateTodoRequestT {
    pub title: Option<String>,
    pub description: Option<String>,
}

impl Default for CreateTodoRequestT {
    fn default() -> Self {
        Self {
            title: None,
            description: None,
        }
    }
}

impl FlatBufferGrpcMessage for CreateTodoRequestT {
    fn encode_flatbuffer(&self) -> Vec<u8> {
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let title = self.title.as_ref().map(|x| builder.create_string(x));
        let description = self.description.as_ref().map(|x| builder.create_string(x));
        let req = FbsCreateTodoRequest::create(
            &mut builder,
            &FbsCreateTodoRequestArgs {
                title,
                description,
            },
        );
        builder.finish(req, None);
        builder.finished_data().to_vec()
    }

    fn decode_flatbuffer(data: &[u8]) -> Result<Self, String> {
        let req = flatbuffers::root::<FbsCreateTodoRequest>(data)
            .map_err(|e| format!("invalid CreateTodoRequest: {e}"))?;
        Ok(CreateTodoRequestT {
            title: req.title().map(|x| x.to_string()),
            description: req.description().map(|x| x.to_string()),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateTodoResponseT {
    pub todo: Option<TodoT>,
}

impl Default for CreateTodoResponseT {
    fn default() -> Self {
        Self { todo: None }
    }
}

impl FlatBufferGrpcMessage for CreateTodoResponseT {
    fn encode_flatbuffer(&self) -> Vec<u8> {
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let todo = self.todo.as_ref().map(|t| {
            let id = t.id.as_ref().map(|x| builder.create_string(x));
            let title = t.title.as_ref().map(|x| builder.create_string(x));
            let description = t.description.as_ref().map(|x| builder.create_string(x));
            FbsTodo::create(&mut builder, &FbsTodoArgs {
                id,
                title,
                description,
                completed: t.completed,
                created_at: t.created_at,
            })
        });
        let resp = FbsCreateTodoResponse::create(
            &mut builder,
            &FbsCreateTodoResponseArgs { todo },
        );
        builder.finish(resp, None);
        builder.finished_data().to_vec()
    }

    fn decode_flatbuffer(data: &[u8]) -> Result<Self, String> {
        let resp = flatbuffers::root::<FbsCreateTodoResponse>(data)
            .map_err(|e| format!("invalid CreateTodoResponse: {e}"))?;
        Ok(CreateTodoResponseT {
            todo: resp.todo().map(|t| TodoT {
                id: t.id().map(|x| x.to_string()),
                title: t.title().map(|x| x.to_string()),
                description: t.description().map(|x| x.to_string()),
                completed: t.completed(),
                created_at: t.created_at(),
            }),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GetTodoRequestT {
    pub id: Option<String>,
}

impl Default for GetTodoRequestT {
    fn default() -> Self {
        Self { id: None }
    }
}

impl FlatBufferGrpcMessage for GetTodoRequestT {
    fn encode_flatbuffer(&self) -> Vec<u8> {
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let id = self.id.as_ref().map(|x| builder.create_string(x));
        let req = FbsGetTodoRequest::create(
            &mut builder,
            &FbsGetTodoRequestArgs { id },
        );
        builder.finish(req, None);
        builder.finished_data().to_vec()
    }

    fn decode_flatbuffer(data: &[u8]) -> Result<Self, String> {
        let req = flatbuffers::root::<FbsGetTodoRequest>(data)
            .map_err(|e| format!("invalid GetTodoRequest: {e}"))?;
        Ok(GetTodoRequestT {
            id: req.id().map(|x| x.to_string()),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GetTodoResponseT {
    pub todo: Option<TodoT>,
}

impl Default for GetTodoResponseT {
    fn default() -> Self {
        Self { todo: None }
    }
}

impl FlatBufferGrpcMessage for GetTodoResponseT {
    fn encode_flatbuffer(&self) -> Vec<u8> {
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let todo = self.todo.as_ref().map(|t| {
            let id = t.id.as_ref().map(|x| builder.create_string(x));
            let title = t.title.as_ref().map(|x| builder.create_string(x));
            let description = t.description.as_ref().map(|x| builder.create_string(x));
            FbsTodo::create(&mut builder, &FbsTodoArgs {
                id,
                title,
                description,
                completed: t.completed,
                created_at: t.created_at,
            })
        });
        let resp = FbsGetTodoResponse::create(
            &mut builder,
            &FbsGetTodoResponseArgs { todo },
        );
        builder.finish(resp, None);
        builder.finished_data().to_vec()
    }

    fn decode_flatbuffer(data: &[u8]) -> Result<Self, String> {
        let resp = flatbuffers::root::<FbsGetTodoResponse>(data)
            .map_err(|e| format!("invalid GetTodoResponse: {e}"))?;
        Ok(GetTodoResponseT {
            todo: resp.todo().map(|t| TodoT {
                id: t.id().map(|x| x.to_string()),
                title: t.title().map(|x| x.to_string()),
                description: t.description().map(|x| x.to_string()),
                completed: t.completed(),
                created_at: t.created_at(),
            }),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ListTodosRequestT;

impl FlatBufferGrpcMessage for ListTodosRequestT {
    fn encode_flatbuffer(&self) -> Vec<u8> {
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let req = FbsListTodosRequest::create(
            &mut builder,
            &FbsListTodosRequestArgs {},
        );
        builder.finish(req, None);
        builder.finished_data().to_vec()
    }

    fn decode_flatbuffer(_data: &[u8]) -> Result<Self, String> {
        Ok(ListTodosRequestT)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListTodosResponseT {
    pub todos: Vec<TodoT>,
}

impl Default for ListTodosResponseT {
    fn default() -> Self {
        Self { todos: Vec::new() }
    }
}

impl FlatBufferGrpcMessage for ListTodosResponseT {
    fn encode_flatbuffer(&self) -> Vec<u8> {
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let todos = self.todos.iter().map(|t| {
            let id = t.id.as_ref().map(|x| builder.create_string(x));
            let title = t.title.as_ref().map(|x| builder.create_string(x));
            let description = t.description.as_ref().map(|x| builder.create_string(x));
            FbsTodo::create(&mut builder, &FbsTodoArgs {
                id,
                title,
                description,
                completed: t.completed,
                created_at: t.created_at,
            })
        }).collect::<Vec<_>>();
        let todos = builder.create_vector(&todos);
        let resp = FbsListTodosResponse::create(
            &mut builder,
            &FbsListTodosResponseArgs { todos: Some(todos) },
        );
        builder.finish(resp, None);
        builder.finished_data().to_vec()
    }

    fn decode_flatbuffer(data: &[u8]) -> Result<Self, String> {
        let resp = flatbuffers::root::<FbsListTodosResponse>(data)
            .map_err(|e| format!("invalid ListTodosResponse: {e}"))?;
        let todos = resp.todos().map(|v| v.iter().map(|t| TodoT {
            id: t.id().map(|x| x.to_string()),
            title: t.title().map(|x| x.to_string()),
            description: t.description().map(|x| x.to_string()),
            completed: t.completed(),
            created_at: t.created_at(),
        }).collect()).unwrap_or_default();
        Ok(ListTodosResponseT { todos })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeleteTodoRequestT {
    pub id: Option<String>,
}

impl Default for DeleteTodoRequestT {
    fn default() -> Self {
        Self { id: None }
    }
}

impl FlatBufferGrpcMessage for DeleteTodoRequestT {
    fn encode_flatbuffer(&self) -> Vec<u8> {
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let id = self.id.as_ref().map(|x| builder.create_string(x));
        let req = FbsDeleteTodoRequest::create(
            &mut builder,
            &FbsDeleteTodoRequestArgs { id },
        );
        builder.finish(req, None);
        builder.finished_data().to_vec()
    }

    fn decode_flatbuffer(data: &[u8]) -> Result<Self, String> {
        let req = flatbuffers::root::<FbsDeleteTodoRequest>(data)
            .map_err(|e| format!("invalid DeleteTodoRequest: {e}"))?;
        Ok(DeleteTodoRequestT {
            id: req.id().map(|x| x.to_string()),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DeleteTodoResponseT;

impl FlatBufferGrpcMessage for DeleteTodoResponseT {
    fn encode_flatbuffer(&self) -> Vec<u8> {
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let resp = FbsDeleteTodoResponse::create(
            &mut builder,
            &FbsDeleteTodoResponseArgs {},
        );
        builder.finish(resp, None);
        builder.finished_data().to_vec()
    }

    fn decode_flatbuffer(_data: &[u8]) -> Result<Self, String> {
        Ok(DeleteTodoResponseT)
    }
}

// ============================================================================
// Tonic Client (manual prost encoding - avoids conflict with grpc_build)
// ============================================================================

use http::uri::PathAndQuery as HttpPathAndQuery;
use tonic::client::Grpc as TonicGrpc;
use tonic::codec::ProstCodec as TonicProstCodec;
use tonic::transport::Channel as TonicChannel;

#[derive(Debug, Clone)]
pub struct TonicTodoClient {
    grpc: TonicGrpc<TonicChannel>,
}

impl TonicTodoClient {
    pub async fn connect(uri: http::Uri) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let channel = tonic::transport::Endpoint::from(uri)
            .connect()
            .await?;
        let grpc = TonicGrpc::new(channel);
        Ok(Self { grpc })
    }

    pub async fn create_todo(
        &mut self,
        title: String,
        description: String,
    ) -> Result<String, tonic::Status> {
        let request = tonic::Request::new(CreateTodoRequest { title, description });
        let path = HttpPathAndQuery::from_static("/todo.TodoService/CreateTodo");
        let codec = TonicProstCodec::<CreateTodoRequest, CreateTodoResponse>::default();

        let response = self.grpc.unary(request, path, codec).await?;
        Ok(response.into_inner().todo.map(|t| t.id).unwrap_or_default())
    }

    pub async fn get_todo(&mut self, id: String) -> Result<(), tonic::Status> {
        let request = tonic::Request::new(GetTodoRequest { id });
        let path = HttpPathAndQuery::from_static("/todo.TodoService/GetTodo");
        let codec = TonicProstCodec::<GetTodoRequest, GetTodoResponse>::default();

        self.grpc.unary(request, path, codec).await?;
        Ok(())
    }

    pub async fn list_todos(&mut self) -> Result<(), tonic::Status> {
        let request = tonic::Request::new(ListTodosRequest {});
        let path = HttpPathAndQuery::from_static("/todo.TodoService/ListTodos");
        let codec = TonicProstCodec::<ListTodosRequest, ListTodosResponse>::default();

        self.grpc.unary(request, path, codec).await?;
        Ok(())
    }

    pub async fn delete_todo(&mut self, id: String) -> Result<(), tonic::Status> {
        let request = tonic::Request::new(DeleteTodoRequest { id });
        let path = HttpPathAndQuery::from_static("/todo.TodoService/DeleteTodo");
        let codec = TonicProstCodec::<DeleteTodoRequest, DeleteTodoResponse>::default();

        self.grpc.unary(request, path, codec).await?;
        Ok(())
    }
}

// ============================================================================
// Prost Client (using pure-grpc with prost codec)
// ============================================================================

#[derive(Debug, Clone)]
pub struct ProstTodoClient {
    grpc: Grpc<Channel>,
}

impl ProstTodoClient {
    pub async fn connect(uri: http::Uri) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let channel = Channel::connect(uri.clone()).await?;
        let grpc = Grpc::with_origin(channel, uri);
        Ok(Self { grpc })
    }

    pub async fn create_todo(
        &mut self,
        title: String,
        description: String,
    ) -> Result<String, Status> {
        let request = ProstCreateTodoRequest { title, description };
        let codec = ProstCodec::<ProstCreateTodoRequest, ProstCreateTodoResponse>::default();
        let path = PathAndQuery::from_static("/todo.TodoService/CreateTodo");

        let mut req = request.into_request();
        req.extensions_mut()
            .insert(GrpcMethod::new("todo.TodoService", "CreateTodo"));

        self.grpc
            .ready()
            .await
            .map_err(|e| Status::unknown(format!("Service was not ready: {}", e)))?;
        let response: Response<ProstCreateTodoResponse> = self.grpc.unary(req, path, codec).await?;
        Ok(response.into_inner().todo.map(|t| t.id).unwrap_or_default())
    }

    pub async fn get_todo(&mut self, id: String) -> Result<(), Status> {
        let request = ProstGetTodoRequest { id };
        let codec = ProstCodec::<ProstGetTodoRequest, ProstGetTodoResponse>::default();
        let path = PathAndQuery::from_static("/todo.TodoService/GetTodo");

        let mut req = request.into_request();
        req.extensions_mut()
            .insert(GrpcMethod::new("todo.TodoService", "GetTodo"));

        self.grpc
            .ready()
            .await
            .map_err(|e| Status::unknown(format!("Service was not ready: {}", e)))?;
        self.grpc.unary(req, path, codec).await?;
        Ok(())
    }

    pub async fn list_todos(&mut self) -> Result<(), Status> {
        let request = ProstListTodosRequest {};
        let codec = ProstCodec::<ProstListTodosRequest, ProstListTodosResponse>::default();
        let path = PathAndQuery::from_static("/todo.TodoService/ListTodos");

        let mut req = request.into_request();
        req.extensions_mut()
            .insert(GrpcMethod::new("todo.TodoService", "ListTodos"));

        self.grpc
            .ready()
            .await
            .map_err(|e| Status::unknown(format!("Service was not ready: {}", e)))?;
        self.grpc.unary(req, path, codec).await?;
        Ok(())
    }

    pub async fn delete_todo(&mut self, id: String) -> Result<(), Status> {
        let request = ProstDeleteTodoRequest { id };
        let codec = ProstCodec::<ProstDeleteTodoRequest, ProstDeleteTodoResponse>::default();
        let path = PathAndQuery::from_static("/todo.TodoService/DeleteTodo");

        let mut req = request.into_request();
        req.extensions_mut()
            .insert(GrpcMethod::new("todo.TodoService", "DeleteTodo"));

        self.grpc
            .ready()
            .await
            .map_err(|e| Status::unknown(format!("Service was not ready: {}", e)))?;
        self.grpc.unary(req, path, codec).await?;
        Ok(())
    }
}

// ============================================================================
// FlatBuffers Client (using pure-grpc with flatbuffers codec)
// ============================================================================

#[derive(Debug, Clone)]
pub struct FlatBuffersTodoClient {
    grpc: Grpc<Channel>,
}

impl FlatBuffersTodoClient {
    pub async fn connect(uri: http::Uri) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let channel = Channel::connect(uri.clone()).await?;
        let grpc = Grpc::with_origin(channel, uri);
        Ok(Self { grpc })
    }

    pub async fn create_todo(
        &mut self,
        title: String,
        description: String,
    ) -> Result<String, Status> {
        let request = CreateTodoRequestT {
            title: Some(title),
            description: Some(description),
        };
        let codec = FlatBuffersCodec::<CreateTodoRequestT, CreateTodoResponseT>::default();
        let path = PathAndQuery::from_static("/todo.TodoService/CreateTodo");

        let mut req = request.into_request();
        req.extensions_mut()
            .insert(GrpcMethod::new("todo.TodoService", "CreateTodo"));

        self.grpc
            .ready()
            .await
            .map_err(|e| Status::unknown(format!("Service was not ready: {}", e)))?;
        let response: Response<CreateTodoResponseT> = self.grpc.unary(req, path, codec).await?;
        Ok(response.into_inner().todo.map(|t| t.id.unwrap_or_default()).unwrap_or_default())
    }

    pub async fn get_todo(&mut self, id: String) -> Result<(), Status> {
        let request = GetTodoRequestT { id: Some(id) };
        let codec = FlatBuffersCodec::<GetTodoRequestT, GetTodoResponseT>::default();
        let path = PathAndQuery::from_static("/todo.TodoService/GetTodo");

        let mut req = request.into_request();
        req.extensions_mut()
            .insert(GrpcMethod::new("todo.TodoService", "GetTodo"));

        self.grpc
            .ready()
            .await
            .map_err(|e| Status::unknown(format!("Service was not ready: {}", e)))?;
        self.grpc.unary(req, path, codec).await?;
        Ok(())
    }

    pub async fn list_todos(&mut self) -> Result<(), Status> {
        let request = ListTodosRequestT;
        let codec = FlatBuffersCodec::<ListTodosRequestT, ListTodosResponseT>::default();
        let path = PathAndQuery::from_static("/todo.TodoService/ListTodos");

        let mut req = request.into_request();
        req.extensions_mut()
            .insert(GrpcMethod::new("todo.TodoService", "ListTodos"));

        self.grpc
            .ready()
            .await
            .map_err(|e| Status::unknown(format!("Service was not ready: {}", e)))?;
        self.grpc.unary(req, path, codec).await?;
        Ok(())
    }

    pub async fn delete_todo(&mut self, id: String) -> Result<(), Status> {
        let request = DeleteTodoRequestT { id: Some(id) };
        let codec = FlatBuffersCodec::<DeleteTodoRequestT, DeleteTodoResponseT>::default();
        let path = PathAndQuery::from_static("/todo.TodoService/DeleteTodo");

        let mut req = request.into_request();
        req.extensions_mut()
            .insert(GrpcMethod::new("todo.TodoService", "DeleteTodo"));

        self.grpc
            .ready()
            .await
            .map_err(|e| Status::unknown(format!("Service was not ready: {}", e)))?;
        self.grpc.unary(req, path, codec).await?;
        Ok(())
    }
}

// ============================================================================
// Load Test Configuration
// ============================================================================

#[derive(Debug, Clone)]
pub struct Config {
    pub duration_secs: u64,
    pub concurrency: u32,
    pub warmup_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            duration_secs: 10,
            concurrency: 50,
            warmup_secs: 1,
        }
    }
}

// ============================================================================
// Load Test Result
// ============================================================================

#[derive(Debug)]
pub struct LoadTestResult {
    pub total_requests: u64,
    pub success_requests: u64,
    pub qps: f64,
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
}

impl LoadTestResult {
    fn success_rate(&self) -> f64 {
        if self.total_requests == 0 {
            0.0
        } else {
            self.success_requests as f64 / self.total_requests as f64 * 100.0
        }
    }

    pub fn print(&self, name: &str) {
        println!("\n=== {} Load Test Results ===", name);
        println!(
            "Total requests: {} ({} success, {:.1}%)",
            self.total_requests,
            self.success_requests,
            self.success_rate()
        );
        println!("QPS: {:.0}", self.qps);
        println!("Latency p50: {:.3}ms", self.p50_ns as f64 / 1_000_000.0);
        println!("Latency p95: {:.3}ms", self.p95_ns as f64 / 1_000_000.0);
        println!("Latency p99: {:.3}ms", self.p99_ns as f64 / 1_000_000.0);
    }
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

// ============================================================================
// Tonic Load Test
// ============================================================================

async fn run_tonic_load_test(
    config: &Config,
    port: u16,
) -> Result<LoadTestResult, Box<dyn std::error::Error + Send + Sync>> {
    let uri: http::Uri = format!("http://127.0.0.1:{}", port).parse()?;

    // Warmup
    let mut warmup_client = TonicTodoClient::connect(uri.clone()).await?;
    let _ = warmup_client.create_todo("warmup".to_string(), "warmup desc".to_string()).await;
    drop(warmup_client);
    tokio::time::sleep(Duration::from_secs(config.warmup_secs)).await;

    // Run load test
    let barrier = Arc::new(Barrier::new(config.concurrency as usize));
    let start = Instant::now();
    let mut total_requests: u64 = 0;
    let mut success_requests: u64 = 0;
    let mut latencies: Vec<u64> = Vec::new();
    let (tx, mut rx) = mpsc::channel::<(u64, u64, Vec<u64>)>((config.concurrency * 2) as usize);

    // Spawn workers - each creates its own connection
    let workers: Vec<_> = (0..config.concurrency)
        .map(|id| {
            let tx = tx.clone();
            let barrier = barrier.clone();
            let config = config.clone();
            let uri = uri.clone();

            tokio::spawn(async move {
                // Each worker creates its own client
                let mut client = TonicTodoClient::connect(uri).await.unwrap();
                let duration = Duration::from_secs(config.duration_secs);

                barrier.wait().await;
                let worker_start = Instant::now();
                let mut local_total: u64 = 0;
                let mut local_success: u64 = 0;
                let mut local_latencies: Vec<u64> = Vec::new();

                while worker_start.elapsed() < duration {
                    // Mix: 20% Create, 40% List, 30% Get, 10% Delete
                    let op = (local_total % 10) as usize;

                    let req_start = Instant::now();
                    let result = match op {
                        0 | 1 => {
                            // Create (20%)
                            let _ = client
                                .create_todo(
                                    format!("todo-{}", worker_start.elapsed().as_nanos()),
                                    "description".to_string(),
                                )
                                .await;
                            Ok(true) // Track created ID
                        }
                        2 | 3 | 4 | 5 => {
                            // List (40%)
                            client.list_todos().await.map(|_| false)
                        }
                        6 | 7 | 8 => {
                            // Get (30%) - use the worker ID as lookup key
                            client
                                .get_todo(format!("todo-{}", id))
                                .await
                                .map(|_| false)
                        }
                        9 => {
                            // Delete (10%)
                            client
                                .delete_todo(format!("todo-{}", id))
                                .await
                                .map(|_| false)
                        }
                        _ => Ok(false),
                    };

                    let latency = req_start.elapsed().as_nanos() as u64;
                    local_total += 1;
                    local_latencies.push(latency);

                    if result.is_ok() {
                        local_success += 1;
                    }
                }

                let _ = tx.send((local_total, local_success, local_latencies)).await;
            })
        })
        .collect();

    // Collect results
    while let Some((total, success, worker_latencies)) = rx.recv().await {
        total_requests += total;
        success_requests += success;
        latencies.extend(worker_latencies);
    }

    for handle in workers {
        let _ = handle.await;
    }

    let elapsed = start.elapsed().as_secs_f64();
    latencies.sort();

    let qps = if elapsed > 0.0 {
        total_requests as f64 / elapsed
    } else {
        0.0
    };

    Ok(LoadTestResult {
        total_requests,
        success_requests,
        qps,
        p50_ns: percentile(&latencies, 0.50),
        p95_ns: percentile(&latencies, 0.95),
        p99_ns: percentile(&latencies, 0.99),
    })
}

// ============================================================================
// Prost Load Test
// ============================================================================

async fn run_prost_load_test(
    config: &Config,
    port: u16,
) -> Result<LoadTestResult, Box<dyn std::error::Error + Send + Sync>> {
    let uri: http::Uri = format!("http://127.0.0.1:{}", port).parse()?;

    // Warmup
    let mut warmup_client = ProstTodoClient::connect(uri.clone()).await?;
    let _ = warmup_client
        .create_todo("warmup".to_string(), "warmup desc".to_string())
        .await;
    drop(warmup_client);
    tokio::time::sleep(Duration::from_secs(config.warmup_secs)).await;

    // Run load test
    let barrier = Arc::new(Barrier::new(config.concurrency as usize));
    let start = Instant::now();
    let mut total_requests: u64 = 0;
    let mut success_requests: u64 = 0;
    let mut latencies: Vec<u64> = Vec::new();
    let (tx, mut rx) = mpsc::channel::<(u64, u64, Vec<u64>)>((config.concurrency * 2) as usize);

    // Spawn workers - each creates its own connection
    let workers: Vec<_> = (0..config.concurrency)
        .map(|id| {
            let tx = tx.clone();
            let barrier = barrier.clone();
            let config = config.clone();
            let uri = uri.clone();

            tokio::spawn(async move {
                // Each worker creates its own client
                let mut client = ProstTodoClient::connect(uri).await.unwrap();
                let duration = Duration::from_secs(config.duration_secs);

                barrier.wait().await;
                let worker_start = Instant::now();
                let mut local_total: u64 = 0;
                let mut local_success: u64 = 0;
                let mut local_latencies: Vec<u64> = Vec::new();

                while worker_start.elapsed() < duration {
                    // Mix: 20% Create, 40% List, 30% Get, 10% Delete
                    let op = (local_total % 10) as usize;

                    let req_start = Instant::now();
                    let result = match op {
                        0 | 1 => {
                            // Create (20%)
                            let _ = client
                                .create_todo(
                                    format!("todo-{}", worker_start.elapsed().as_nanos()),
                                    "description".to_string(),
                                )
                                .await;
                            Ok(true)
                        }
                        2 | 3 | 4 | 5 => {
                            // List (40%)
                            client.list_todos().await.map(|_| false)
                        }
                        6 | 7 | 8 => {
                            // Get (30%)
                            client
                                .get_todo(format!("todo-{}", id))
                                .await
                                .map(|_| false)
                        }
                        9 => {
                            // Delete (10%)
                            client
                                .delete_todo(format!("todo-{}", id))
                                .await
                                .map(|_| false)
                        }
                        _ => Ok(false),
                    };

                    let latency = req_start.elapsed().as_nanos() as u64;
                    local_total += 1;
                    local_latencies.push(latency);

                    if result.is_ok() {
                        local_success += 1;
                    }
                }

                let _ = tx.send((local_total, local_success, local_latencies)).await;
            })
        })
        .collect();

    // Collect results
    while let Some((total, success, worker_latencies)) = rx.recv().await {
        total_requests += total;
        success_requests += success;
        latencies.extend(worker_latencies);
    }

    for handle in workers {
        let _ = handle.await;
    }

    let elapsed = start.elapsed().as_secs_f64();
    latencies.sort();

    let qps = if elapsed > 0.0 {
        total_requests as f64 / elapsed
    } else {
        0.0
    };

    Ok(LoadTestResult {
        total_requests,
        success_requests,
        qps,
        p50_ns: percentile(&latencies, 0.50),
        p95_ns: percentile(&latencies, 0.95),
        p99_ns: percentile(&latencies, 0.99),
    })
}

// ============================================================================
// FlatBuffers Load Test
// ============================================================================

async fn run_flatbuffers_load_test(
    config: &Config,
    port: u16,
) -> Result<LoadTestResult, Box<dyn std::error::Error + Send + Sync>> {
    let uri: http::Uri = format!("http://127.0.0.1:{}", port).parse()?;

    // Warmup
    let mut warmup_client = FlatBuffersTodoClient::connect(uri.clone()).await?;
    let _ = warmup_client
        .create_todo("warmup".to_string(), "warmup desc".to_string())
        .await;
    drop(warmup_client);
    tokio::time::sleep(Duration::from_secs(config.warmup_secs)).await;

    // Run load test
    let barrier = Arc::new(Barrier::new(config.concurrency as usize));
    let start = Instant::now();
    let mut total_requests: u64 = 0;
    let mut success_requests: u64 = 0;
    let mut latencies: Vec<u64> = Vec::new();
    let (tx, mut rx) = mpsc::channel::<(u64, u64, Vec<u64>)>((config.concurrency * 2) as usize);

    // Spawn workers - each creates its own connection
    let workers: Vec<_> = (0..config.concurrency)
        .map(|id| {
            let tx = tx.clone();
            let barrier = barrier.clone();
            let config = config.clone();
            let uri = uri.clone();

            tokio::spawn(async move {
                // Each worker creates its own client
                let mut client = FlatBuffersTodoClient::connect(uri).await.unwrap();
                let duration = Duration::from_secs(config.duration_secs);

                barrier.wait().await;
                let worker_start = Instant::now();
                let mut local_total: u64 = 0;
                let mut local_success: u64 = 0;
                let mut local_latencies: Vec<u64> = Vec::new();

                while worker_start.elapsed() < duration {
                    // Mix: 20% Create, 40% List, 30% Get, 10% Delete
                    let op = (local_total % 10) as usize;

                    let req_start = Instant::now();
                    let result = match op {
                        0 | 1 => {
                            // Create (20%)
                            let _ = client
                                .create_todo(
                                    format!("todo-{}", worker_start.elapsed().as_nanos()),
                                    "description".to_string(),
                                )
                                .await;
                            Ok(true)
                        }
                        2 | 3 | 4 | 5 => {
                            // List (40%)
                            client.list_todos().await.map(|_| false)
                        }
                        6 | 7 | 8 => {
                            // Get (30%)
                            client
                                .get_todo(format!("todo-{}", id))
                                .await
                                .map(|_| false)
                        }
                        9 => {
                            // Delete (10%)
                            client
                                .delete_todo(format!("todo-{}", id))
                                .await
                                .map(|_| false)
                        }
                        _ => Ok(false),
                    };

                    let latency = req_start.elapsed().as_nanos() as u64;
                    local_total += 1;
                    local_latencies.push(latency);

                    if result.is_ok() {
                        local_success += 1;
                    }
                }

                let _ = tx.send((local_total, local_success, local_latencies)).await;
            })
        })
        .collect();

    // Collect results
    while let Some((total, success, worker_latencies)) = rx.recv().await {
        total_requests += total;
        success_requests += success;
        latencies.extend(worker_latencies);
    }

    for handle in workers {
        let _ = handle.await;
    }

    let elapsed = start.elapsed().as_secs_f64();
    latencies.sort();

    let qps = if elapsed > 0.0 {
        total_requests as f64 / elapsed
    } else {
        0.0
    };

    Ok(LoadTestResult {
        total_requests,
        success_requests,
        qps,
        p50_ns: percentile(&latencies, 0.50),
        p95_ns: percentile(&latencies, 0.95),
        p99_ns: percentile(&latencies, 0.99),
    })
}

// ============================================================================
// Main
// ============================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    #[derive(Parser, Debug)]
    struct Args {
        #[arg(long, default_value_t = 10)]
        duration: u64,

        #[arg(long, default_value_t = 50)]
        concurrency: u32,

        #[arg(long, default_value_t = 50051)]
        flatbuffers_port: u16,

        #[arg(long, default_value_t = 50052)]
        prost_port: u16,

        #[arg(long, default_value_t = 50053)]
        tonic_port: u16,
    }

    let args = Args::parse();

    let config = Config {
        duration_secs: args.duration,
        concurrency: args.concurrency,
        warmup_secs: 1,
    };

    println!("gRPC Todo Service Load Test Configuration:");
    println!("  Duration: {}s", config.duration_secs);
    println!("  Concurrency: {}", config.concurrency);
    println!();
    println!("This load test requires running servers:");
    println!("  1. FlatBuffers server on port {}", args.flatbuffers_port);
    println!("     cargo run --release -p benchmarks --bin flatbuffers_todo_server -- --port {}", args.flatbuffers_port);
    println!("  2. Prost (pure-grpc) server on port {}", args.prost_port);
    println!("     cargo run --release -p benchmarks --bin prost_todo_server -- --port {}", args.prost_port);
    println!("  3. Tonic server on port {}", args.tonic_port);
    println!("     cargo run --release -p tonic-bench-server --bin tonic_todo_server -- --port {}", args.tonic_port);
    println!();

    // Run FlatBuffers load test
    println!("\n--- FlatBuffers (pure-grpc) Load Test ---");
    match timeout(
        Duration::from_secs(config.duration_secs + 10),
        run_flatbuffers_load_test(&config, args.flatbuffers_port),
    )
    .await
    {
        Ok(Ok(result)) => result.print("FlatBuffers (pure-grpc)"),
        Ok(Err(e)) => println!("FlatBuffers load test failed: {}", e),
        Err(_) => println!("FlatBuffers load test timed out (server not running?)"),
    }

    // Run Prost load test
    println!("\n--- Prost (pure-grpc + prost) Load Test ---");
    match timeout(
        Duration::from_secs(config.duration_secs + 10),
        run_prost_load_test(&config, args.prost_port),
    )
    .await
    {
        Ok(Ok(result)) => result.print("Prost (pure-grpc+prost)"),
        Ok(Err(e)) => println!("Prost load test failed: {}", e),
        Err(_) => println!("Prost load test timed out (server not running?)"),
    }

    // Run Tonic load test
    println!("\n--- Tonic (prost) Load Test ---");
    match timeout(
        Duration::from_secs(config.duration_secs + 10),
        run_tonic_load_test(&config, args.tonic_port),
    )
    .await
    {
        Ok(Ok(result)) => result.print("Tonic (prost)"),
        Ok(Err(e)) => println!("Tonic load test failed: {}", e),
        Err(_) => println!("Tonic load test timed out (server not running?)"),
    }

    println!();

    Ok(())
}
