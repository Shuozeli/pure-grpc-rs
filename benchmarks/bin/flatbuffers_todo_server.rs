//! FlatBuffers-based Todo Service server using pure-grpc-rs
//!
//! Run with: `cargo run --release -p benchmarks --bin flatbuffers_todo_server -- --port 50051`

#![allow(clippy::all, unused, dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;

use clap::Parser;
use tokio::sync::RwLock;
use uuid::Uuid;

// Include generated FlatBuffers code
// We include todo_generated.rs but todo.rs is also compiled and generates warnings
// Use clippy::all to suppress all clippy warnings
#[allow(clippy::all, unused, dead_code, non_snake_case)]
mod generated {
    #![allow(clippy::all, unused, dead_code, unused_imports)]
    include!(concat!(env!("OUT_DIR"), "/todo_generated.rs"));
}

// Also include todo.rs to suppress its warnings
#[allow(clippy::all, unused, dead_code, unused_imports)]
mod todo_types {
    #![allow(clippy::all, unused, dead_code, unused_imports)]
    include!(concat!(env!("OUT_DIR"), "/todo.rs"));
}

// Suppress derivable impls warning for hand-written Default impls
#[allow(clippy::derivable_impls)]
// Import FlatBuffers table types from generated code
use generated::todo::{
    CreateTodoRequest as FbsCreateTodoRequest, CreateTodoRequestArgs as FbsCreateTodoRequestArgs,
    CreateTodoResponse as FbsCreateTodoResponse,
    CreateTodoResponseArgs as FbsCreateTodoResponseArgs, DeleteTodoRequest as FbsDeleteTodoRequest,
    DeleteTodoRequestArgs as FbsDeleteTodoRequestArgs, DeleteTodoResponse as FbsDeleteTodoResponse,
    DeleteTodoResponseArgs as FbsDeleteTodoResponseArgs, GetTodoRequest as FbsGetTodoRequest,
    GetTodoRequestArgs as FbsGetTodoRequestArgs, GetTodoResponse as FbsGetTodoResponse,
    GetTodoResponseArgs as FbsGetTodoResponseArgs, ListTodosRequest as FbsListTodosRequest,
    ListTodosRequestArgs as FbsListTodosRequestArgs, ListTodosResponse as FbsListTodosResponse,
    ListTodosResponseArgs as FbsListTodosResponseArgs, Todo as FbsTodo, TodoArgs as FbsTodoArgs,
};

use grpc_codec_flatbuffers::{FlatBufferGrpcMessage, FlatBuffersCodec};
use grpc_core::body::Body;
use grpc_core::{BoxFuture, Request, Response, Status};
use grpc_server::{Grpc, NamedService, Router, Server, UnaryService};
use std::convert::Infallible;
use std::task::{Context, Poll};

// --- Owned message types that implement FlatBufferGrpcMessage ---

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
        let req = flatbuffers::root::<FbsTodo>(data).map_err(|e| format!("invalid Todo: {e}"))?;
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
            &FbsCreateTodoRequestArgs { title, description },
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
            FbsTodo::create(
                &mut builder,
                &FbsTodoArgs {
                    id,
                    title,
                    description,
                    completed: t.completed,
                    created_at: t.created_at,
                },
            )
        });
        let resp = FbsCreateTodoResponse::create(&mut builder, &FbsCreateTodoResponseArgs { todo });
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
        let req = FbsGetTodoRequest::create(&mut builder, &FbsGetTodoRequestArgs { id });
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
            FbsTodo::create(
                &mut builder,
                &FbsTodoArgs {
                    id,
                    title,
                    description,
                    completed: t.completed,
                    created_at: t.created_at,
                },
            )
        });
        let resp = FbsGetTodoResponse::create(&mut builder, &FbsGetTodoResponseArgs { todo });
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
        let req = FbsListTodosRequest::create(&mut builder, &FbsListTodosRequestArgs {});
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
        let todos = self
            .todos
            .iter()
            .map(|t| {
                let id = t.id.as_ref().map(|x| builder.create_string(x));
                let title = t.title.as_ref().map(|x| builder.create_string(x));
                let description = t.description.as_ref().map(|x| builder.create_string(x));
                FbsTodo::create(
                    &mut builder,
                    &FbsTodoArgs {
                        id,
                        title,
                        description,
                        completed: t.completed,
                        created_at: t.created_at,
                    },
                )
            })
            .collect::<Vec<_>>();
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
        let todos = resp
            .todos()
            .map(|v| {
                v.iter()
                    .map(|t| TodoT {
                        id: t.id().map(|x| x.to_string()),
                        title: t.title().map(|x| x.to_string()),
                        description: t.description().map(|x| x.to_string()),
                        completed: t.completed(),
                        created_at: t.created_at(),
                    })
                    .collect()
            })
            .unwrap_or_default();
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
        let req = FbsDeleteTodoRequest::create(&mut builder, &FbsDeleteTodoRequestArgs { id });
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
        let resp = FbsDeleteTodoResponse::create(&mut builder, &FbsDeleteTodoResponseArgs {});
        builder.finish(resp, None);
        builder.finished_data().to_vec()
    }

    fn decode_flatbuffer(_data: &[u8]) -> Result<Self, String> {
        Ok(DeleteTodoResponseT)
    }
}

// --- Service Implementation ---

#[derive(Clone)]
struct TodoServiceImpl {
    todos: Arc<RwLock<std::collections::HashMap<String, TodoT>>>,
}

impl TodoServiceImpl {
    fn new() -> Self {
        Self {
            todos: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }
}

impl TodoService for TodoServiceImpl {
    fn create_todo(
        &self,
        request: Request<CreateTodoRequestT>,
    ) -> BoxFuture<Result<Response<CreateTodoResponseT>, Status>> {
        let req = request.into_inner();
        let todos = self.todos.clone();

        Box::pin(async move {
            let id = Uuid::new_v4().to_string();
            let created_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;

            let todo = TodoT {
                id: Some(id.clone()),
                title: req.title,
                description: req.description,
                completed: false,
                created_at,
            };

            let mut todos = todos.write().await;
            todos.insert(id, todo.clone());

            Ok(Response::new(CreateTodoResponseT { todo: Some(todo) }))
        })
    }

    fn get_todo(
        &self,
        request: Request<GetTodoRequestT>,
    ) -> BoxFuture<Result<Response<GetTodoResponseT>, Status>> {
        let req = request.into_inner();
        let todos = self.todos.clone();

        Box::pin(async move {
            let todos = todos.read().await;
            match todos.get(req.id.as_ref().unwrap_or(&String::new())) {
                Some(todo) => Ok(Response::new(GetTodoResponseT {
                    todo: Some(todo.clone()),
                })),
                None => Err(Status::not_found("Todo not found")),
            }
        })
    }

    fn list_todos(
        &self,
        _request: Request<ListTodosRequestT>,
    ) -> BoxFuture<Result<Response<ListTodosResponseT>, Status>> {
        let todos = self.todos.clone();

        Box::pin(async move {
            let todos = todos.read().await;
            let todo_list: Vec<TodoT> = todos.values().cloned().collect();
            Ok(Response::new(ListTodosResponseT { todos: todo_list }))
        })
    }

    fn delete_todo(
        &self,
        request: Request<DeleteTodoRequestT>,
    ) -> BoxFuture<Result<Response<DeleteTodoResponseT>, Status>> {
        let req = request.into_inner();
        let todos = self.todos.clone();

        Box::pin(async move {
            let mut todos = todos.write().await;
            if todos
                .remove(req.id.as_ref().unwrap_or(&String::new()))
                .is_some()
            {
                Ok(Response::new(DeleteTodoResponseT))
            } else {
                Err(Status::not_found("Todo not found"))
            }
        })
    }
}

// --- Server Stubs ---

pub trait TodoService: Send + Sync + 'static {
    fn create_todo(
        &self,
        request: Request<CreateTodoRequestT>,
    ) -> BoxFuture<Result<Response<CreateTodoResponseT>, Status>>;
    fn get_todo(
        &self,
        request: Request<GetTodoRequestT>,
    ) -> BoxFuture<Result<Response<GetTodoResponseT>, Status>>;
    fn list_todos(
        &self,
        request: Request<ListTodosRequestT>,
    ) -> BoxFuture<Result<Response<ListTodosResponseT>, Status>>;
    fn delete_todo(
        &self,
        request: Request<DeleteTodoRequestT>,
    ) -> BoxFuture<Result<Response<DeleteTodoResponseT>, Status>>;
}

pub struct TodoServiceServer<T> {
    inner: Arc<T>,
}

impl<T> Clone for TodoServiceServer<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T: TodoService> TodoServiceServer<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner: Arc::new(inner),
        }
    }
}

impl<T: TodoService> NamedService for TodoServiceServer<T> {
    const NAME: &'static str = "todo.TodoService";
}

struct CreateTodoSvc<T: TodoService>(Arc<T>);
struct GetTodoSvc<T: TodoService>(Arc<T>);
struct ListTodosSvc<T: TodoService>(Arc<T>);
struct DeleteTodoSvc<T: TodoService>(Arc<T>);

impl<T: TodoService> UnaryService<CreateTodoRequestT> for CreateTodoSvc<T> {
    type Response = CreateTodoResponseT;
    type Future = BoxFuture<Result<Response<CreateTodoResponseT>, Status>>;
    fn call(&mut self, request: Request<CreateTodoRequestT>) -> Self::Future {
        let inner = Arc::clone(&self.0);
        Box::pin(async move { inner.create_todo(request).await })
    }
}

impl<T: TodoService> UnaryService<GetTodoRequestT> for GetTodoSvc<T> {
    type Response = GetTodoResponseT;
    type Future = BoxFuture<Result<Response<GetTodoResponseT>, Status>>;
    fn call(&mut self, request: Request<GetTodoRequestT>) -> Self::Future {
        let inner = Arc::clone(&self.0);
        Box::pin(async move { inner.get_todo(request).await })
    }
}

impl<T: TodoService> UnaryService<ListTodosRequestT> for ListTodosSvc<T> {
    type Response = ListTodosResponseT;
    type Future = BoxFuture<Result<Response<ListTodosResponseT>, Status>>;
    fn call(&mut self, request: Request<ListTodosRequestT>) -> Self::Future {
        let inner = Arc::clone(&self.0);
        Box::pin(async move { inner.list_todos(request).await })
    }
}

impl<T: TodoService> UnaryService<DeleteTodoRequestT> for DeleteTodoSvc<T> {
    type Response = DeleteTodoResponseT;
    type Future = BoxFuture<Result<Response<DeleteTodoResponseT>, Status>>;
    fn call(&mut self, request: Request<DeleteTodoRequestT>) -> Self::Future {
        let inner = Arc::clone(&self.0);
        Box::pin(async move { inner.delete_todo(request).await })
    }
}

impl<T: TodoService> tower_service::Service<http::Request<Body>> for TodoServiceServer<T> {
    type Response = http::Response<Body>;
    type Error = Infallible;
    type Future = BoxFuture<Result<http::Response<Body>, Infallible>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: http::Request<Body>) -> Self::Future {
        let inner = self.inner.clone();
        match req.uri().path() {
            "/todo.TodoService/CreateTodo" => Box::pin(async move {
                let mut grpc = Grpc::new(
                    FlatBuffersCodec::<CreateTodoResponseT, CreateTodoRequestT>::default(),
                );
                Ok(grpc.unary(CreateTodoSvc(inner), req).await)
            }),
            "/todo.TodoService/GetTodo" => Box::pin(async move {
                let mut grpc =
                    Grpc::new(FlatBuffersCodec::<GetTodoResponseT, GetTodoRequestT>::default());
                Ok(grpc.unary(GetTodoSvc(inner), req).await)
            }),
            "/todo.TodoService/ListTodos" => Box::pin(async move {
                let mut grpc =
                    Grpc::new(FlatBuffersCodec::<ListTodosResponseT, ListTodosRequestT>::default());
                Ok(grpc.unary(ListTodosSvc(inner), req).await)
            }),
            "/todo.TodoService/DeleteTodo" => Box::pin(async move {
                let mut grpc = Grpc::new(
                    FlatBuffersCodec::<DeleteTodoResponseT, DeleteTodoRequestT>::default(),
                );
                Ok(grpc.unary(DeleteTodoSvc(inner), req).await)
            }),
            _ => Box::pin(async move { Ok(Status::unimplemented("method not found").into_http()) }),
        }
    }
}

#[derive(Parser, Debug)]
struct Args {
    #[arg(long, default_value_t = 50051)]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Args::parse();
    let addr: SocketAddr = format!("127.0.0.1:{}", args.port).parse()?;

    let todo_service = TodoServiceServer::new(TodoServiceImpl::new());

    let router =
        Router::new().add_service(TodoServiceServer::<TodoServiceImpl>::NAME, todo_service);

    println!("Server listening on port {}", args.port);

    Server::builder()
        .serve_with_shutdown(addr, router, async {
            tokio::signal::ctrl_c().await.ok();
            println!("\nShutting down...");
        })
        .await?;

    Ok(())
}
