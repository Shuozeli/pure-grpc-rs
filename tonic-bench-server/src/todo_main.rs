//! Tonic Todo Service server
//!
//! Run with: `cargo run --release -p tonic-bench-server --bin tonic_todo_server -- --port 50053`

use std::net::SocketAddr;
use std::sync::Arc;

use clap::Parser;
use tokio::sync::RwLock;
use tonic::{Request, Response, Status};
use uuid::Uuid;

pub mod todo_service {
    tonic::include_proto!("todo");
}

use todo_service::todo_service_server::TodoService;
use todo_service::{
    CreateTodoRequest, CreateTodoResponse, DeleteTodoRequest, DeleteTodoResponse, GetTodoRequest,
    GetTodoResponse, ListTodosRequest, ListTodosResponse, Todo,
};

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

#[tonic::async_trait]
impl TodoService for TodoServiceImpl {
    async fn create_todo(
        &self,
        request: Request<CreateTodoRequest>,
    ) -> Result<Response<CreateTodoResponse>, Status> {
        let req = request.into_inner();
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

        let mut todos = self.todos.write().await;
        todos.insert(id, todo.clone());

        Ok(Response::new(CreateTodoResponse { todo: Some(todo) }))
    }

    async fn get_todo(
        &self,
        request: Request<GetTodoRequest>,
    ) -> Result<Response<GetTodoResponse>, Status> {
        let req = request.into_inner();
        let todos = self.todos.read().await;

        match todos.get(&req.id) {
            Some(todo) => Ok(Response::new(GetTodoResponse {
                todo: Some(todo.clone()),
            })),
            None => Err(Status::not_found("Todo not found")),
        }
    }

    async fn list_todos(
        &self,
        _request: Request<ListTodosRequest>,
    ) -> Result<Response<ListTodosResponse>, Status> {
        let todos = self.todos.read().await;
        let todo_list: Vec<Todo> = todos.values().cloned().collect();
        Ok(Response::new(ListTodosResponse { todos: todo_list }))
    }

    async fn delete_todo(
        &self,
        request: Request<DeleteTodoRequest>,
    ) -> Result<Response<DeleteTodoResponse>, Status> {
        let req = request.into_inner();
        let mut todos = self.todos.write().await;

        if todos.remove(&req.id).is_some() {
            Ok(Response::new(DeleteTodoResponse {}))
        } else {
            Err(Status::not_found("Todo not found"))
        }
    }
}

#[derive(Parser, Debug)]
struct Args {
    #[arg(long, default_value_t = 50053)]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Args::parse();
    let addr: SocketAddr = format!("127.0.0.1:{}", args.port).parse()?;

    println!("Server listening on port {}", args.port);

    tonic::transport::Server::builder()
        .add_service(todo_service::todo_service_server::TodoServiceServer::new(
            TodoServiceImpl::new(),
        ))
        .serve(addr)
        .await?;

    Ok(())
}
