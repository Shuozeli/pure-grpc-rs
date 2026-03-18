use greeter_proto::greeter_client::GreeterClient;
use greeter_proto::HelloRequest;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();

    let uri: http::Uri = "http://127.0.0.1:50051".parse()?;
    let mut client = GreeterClient::connect(uri).await?;

    // 1. Unary
    println!("--- Unary ---");
    let response = client
        .say_hello(HelloRequest {
            name: "unary-test".into(),
        })
        .await?;
    println!("  Response: {}", response.get_ref().message);

    // 2. Server streaming
    println!("--- Server Streaming ---");
    let response = client
        .say_hello_server_stream(HelloRequest {
            name: "stream-test".into(),
        })
        .await?;
    let mut stream = response.into_inner();
    while let Some(reply) = stream.message().await? {
        println!("  Response: {}", reply.message);
    }

    // 3. Client streaming
    println!("--- Client Streaming ---");
    let names = vec![
        HelloRequest {
            name: "Alice".into(),
        },
        HelloRequest { name: "Bob".into() },
        HelloRequest {
            name: "Charlie".into(),
        },
    ];
    let response = client
        .say_hello_client_stream(tokio_stream::iter(names))
        .await?;
    println!("  Response: {}", response.get_ref().message);

    // 4. Bidirectional streaming
    println!("--- Bidi Streaming ---");
    let names = vec![
        HelloRequest {
            name: "Dave".into(),
        },
        HelloRequest { name: "Eve".into() },
    ];
    let response = client
        .say_hello_bidi_stream(tokio_stream::iter(names))
        .await?;
    let mut stream = response.into_inner();
    while let Some(reply) = stream.message().await? {
        println!("  Response: {}", reply.message);
    }

    println!("--- All patterns succeeded ---");

    Ok(())
}
