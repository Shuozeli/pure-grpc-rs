use greeter_fbs::greeter_client::GreeterClient;
use greeter_fbs::HelloRequest;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();

    let uri: http::Uri = "http://127.0.0.1:50053".parse()?;
    let mut client = GreeterClient::connect(uri).await?;

    // Owned wrapper types are `#[non_exhaustive]` (mirroring the upstream
    // Object API), so populate via Default + field assignment.
    let mut request = HelloRequest::default();
    request.name = Some("FlatBuffers-client".into());

    println!("Sending FlatBuffers request: {:?}", request);
    let response = client.say_hello(request).await?;
    println!("Response: {:?}", response.get_ref());

    Ok(())
}
