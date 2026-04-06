//! Prost-based Todo Service server using pure-grpc-rs
//!
//! Run with: `cargo run --release -p benchmarks --bin prost_todo_server -- --port 50052`

use std::net::SocketAddr;
use std::sync::Arc;

use clap::Parser;
use tokio::sync::RwLock;
use uuid::Uuid;

// Include generated proto code
#[allow(clippy::all, unused, dead_code, unused_imports)]
#[allow(non_snake_case)]
mod generated {
    #![allow(clippy::all, unused, dead_code, unused_imports)]
    include!(concat!(env!("OUT_DIR"), "/todo.rs"));
}

use generated::todo_service_server::{TodoService, TodoServiceServer};
use generated::{
    CreateTodoRequest, CreateTodoResponse, DeleteTodoRequest, DeleteTodoResponse, GetTodoRequest,
    GetTodoResponse, ListTodosRequest, ListTodosResponse, Todo,
};
use grpc_server::{NamedService, Router, Server};
use grpc_core::{Request, Response, Status};

#[derive(Clone)]
struct TodoServiceImpl {
    todos: Arc<RwLock<std::collections::HashMap<String, Todo>>>,
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
        request: Request<CreateTodoRequest>,
    ) -> grpc_core::BoxFuture<Result<Response<CreateTodoResponse>, Status>> {
        let req = request.into_inner();
        let todos = self.todos.clone();

        Box::pin(async move {
            let id = Uuid::new_v4().to_string();
            let created_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;

            let todo = Todo {
                id: id.clone(),
                title: req.title,
                description: req.description,
                completed: false,
                created_at,
            };

            let mut todos = todos.write().await;
            todos.insert(id, todo.clone());

            Ok(Response::new(CreateTodoResponse { todo: Some(todo) }))
        })
    }

    fn get_todo(
        &self,
        request: Request<GetTodoRequest>,
    ) -> grpc_core::BoxFuture<Result<Response<GetTodoResponse>, Status>> {
        let req = request.into_inner();
        let todos = self.todos.clone();

        Box::pin(async move {
            let todos = todos.read().await;
            match todos.get(&req.id) {
                Some(todo) => Ok(Response::new(GetTodoResponse {
                    todo: Some(todo.clone()),
                })),
                None => Err(Status::not_found("Todo not found")),
            }
        })
    }

    fn list_todos(
        &self,
        _request: Request<ListTodosRequest>,
    ) -> grpc_core::BoxFuture<Result<Response<ListTodosResponse>, Status>> {
        let todos = self.todos.clone();

        Box::pin(async move {
            let todos = todos.read().await;
            let todo_list: Vec<Todo> = todos.values().cloned().collect();
            Ok(Response::new(ListTodosResponse { todos: todo_list }))
        })
    }

    fn delete_todo(
        &self,
        request: Request<DeleteTodoRequest>,
    ) -> grpc_core::BoxFuture<Result<Response<DeleteTodoResponse>, Status>> {
        let req = request.into_inner();
        let todos = self.todos.clone();

        Box::pin(async move {
            let mut todos = todos.write().await;
            if todos.remove(&req.id).is_some() {
                Ok(Response::new(DeleteTodoResponse {}))
            } else {
                Err(Status::not_found("Todo not found"))
            }
        })
    }
}

#[derive(Parser, Debug)]
struct Args {
    #[arg(long, default_value_t = 50052)]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Args::parse();
    let addr: SocketAddr = format!("127.0.0.1:{}", args.port).parse()?;

    let todo_service = TodoServiceServer::new(TodoServiceImpl::new());

    let router = Router::new().add_service(TodoServiceServer::<TodoServiceImpl>::NAME, todo_service);

    println!("Server listening on port {}", args.port);

    Server::builder()
        .serve_with_shutdown(addr, router, async {
            tokio::signal::ctrl_c().await.ok();
            println!("\nShutting down...");
        })
        .await?;

    Ok(())
}