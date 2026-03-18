use greeter_proto::greeter_client::GreeterClient;
use greeter_proto::HelloRequest;
use grpc_client::Channel;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();

    let ca_pem = std::fs::read("examples/greeter-proto/certs/ca.crt")?;
    let uri: http::Uri = "https://localhost:50052".parse()?;

    let channel = Channel::connect_with_ca(uri.clone(), &ca_pem).await?;
    let mut client = GreeterClient::with_origin(channel, uri);

    let request = HelloRequest {
        name: "TLS-client".to_string(),
    };

    println!("Sending TLS request: {:?}", request);
    let response = client.say_hello(request).await?;
    println!("Response: {:?}", response.get_ref());

    Ok(())
}
