//! End-to-end tests: real generated Greeter client/server, full protobuf
//! codec, real TCP+TLS handshake, mTLS client-certificate verification.
//!
//! Complements grpc-server's own mtls_integration.rs (which tests the
//! transport layer directly against a raw probe service) by proving the
//! verified client identity survives the whole stack: TLS handshake ->
//! hyper -> grpc dispatch -> codec decode -> generated `Greeter` handler,
//! and back out through a real generated client stub.
#![cfg(feature = "tls")]

use greeter_proto::greeter_client::GreeterClient;
use greeter_proto::greeter_server::{Greeter, GreeterServer};
use greeter_proto::{HelloReply, HelloRequest};
use grpc_client::Channel;
use grpc_core::{BoxFuture, BoxStream, Request, Response, Status, Streaming};
use grpc_server::{ClientAuth, NamedService, PeerCertificates, Router, Server};
use rcgen::{BasicConstraints, Certificate, CertificateParams, DnType, IsCa, KeyPair};
use std::net::SocketAddr;
use tokio::net::TcpListener;

// --- Test service: echoes the verified peer-cert identity back in the reply ---

struct TestGreeter;

fn peer_cert_tag(request: &Request<HelloRequest>) -> String {
    match request
        .extensions()
        .get::<PeerCertificates>()
        .and_then(PeerCertificates::leaf)
    {
        Some(leaf) => format!("cert:{}", hex_encode(leaf)),
        None => "cert:none".to_string(),
    }
}

impl Greeter for TestGreeter {
    fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> BoxFuture<Result<Response<HelloReply>, Status>> {
        let tag = peer_cert_tag(&request);
        let name = request.into_inner().name;
        Box::pin(async move {
            Ok(Response::new(HelloReply {
                message: format!("Hello, {name}! [{tag}]"),
            }))
        })
    }

    type SayHelloServerResponseStream = BoxStream<Result<HelloReply, Status>>;

    fn say_hello_server_stream(
        &self,
        request: Request<HelloRequest>,
    ) -> BoxFuture<Result<Response<Self::SayHelloServerResponseStream>, Status>> {
        let tag = peer_cert_tag(&request);
        let name = request.into_inner().name;
        Box::pin(async move {
            let stream = tokio_stream::iter((0..2).map(move |i| {
                Ok(HelloReply {
                    message: format!("{name}-{i} [{tag}]"),
                })
            }));
            Ok(Response::new(Box::pin(stream) as BoxStream<_>))
        })
    }

    fn say_hello_client_stream(
        &self,
        _request: Request<Streaming<HelloRequest>>,
    ) -> BoxFuture<Result<Response<HelloReply>, Status>> {
        Box::pin(async { Err(Status::unimplemented("not used in mTLS e2e tests")) })
    }

    type SayHelloBidiResponseStream = BoxStream<Result<HelloReply, Status>>;

    fn say_hello_bidi_stream(
        &self,
        _request: Request<Streaming<HelloRequest>>,
    ) -> BoxFuture<Result<Response<Self::SayHelloBidiResponseStream>, Status>> {
        Box::pin(async { Err(Status::unimplemented("not used in mTLS e2e tests")) })
    }
}

// --- Cert generation helpers ---

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn generate_ca(cn: &str) -> (Certificate, KeyPair, Vec<u8>) {
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.distinguished_name.push(DnType::CommonName, cn);
    let key = KeyPair::generate().unwrap();
    let cert = params.self_signed(&key).unwrap();
    let pem = cert.pem().into_bytes();
    (cert, key, pem)
}

/// Returns (cert_pem, key_pem, cert_der).
fn generate_leaf(
    ca_cert: &Certificate,
    ca_key: &KeyPair,
    cn: &str,
    sans: Vec<String>,
) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut params = CertificateParams::new(sans).unwrap();
    params.distinguished_name.push(DnType::CommonName, cn);
    let key = KeyPair::generate().unwrap();
    let cert = params.signed_by(&key, ca_cert, ca_key).unwrap();
    (
        cert.pem().into_bytes(),
        key.serialize_pem().into_bytes(),
        cert.der().as_ref().to_vec(),
    )
}

fn client_tls_config(
    server_ca_pem: &[u8],
    client_cert_pem: Option<(&[u8], &[u8])>,
) -> rustls::ClientConfig {
    let mut roots = rustls::RootCertStore::empty();
    for cert in rustls_pemfile::certs(&mut std::io::BufReader::new(server_ca_pem)) {
        roots.add(cert.unwrap()).unwrap();
    }
    let builder = rustls::ClientConfig::builder().with_root_certificates(roots);

    match client_cert_pem {
        Some((cert_pem, key_pem)) => {
            let certs = rustls_pemfile::certs(&mut std::io::BufReader::new(cert_pem))
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            let key = rustls_pemfile::private_key(&mut std::io::BufReader::new(key_pem))
                .unwrap()
                .unwrap();
            builder.with_client_auth_cert(certs, key).unwrap()
        }
        None => builder.with_no_client_auth(),
    }
}

// --- Server/client setup ---

async fn wait_for_server(addr: SocketAddr) {
    for _ in 0..200 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("server at {addr} did not become ready within 2s");
}

async fn start_mtls_server(
    server_cert_pem: &[u8],
    server_key_pem: &[u8],
    client_ca_pem: &[u8],
    mode: ClientAuth,
) -> SocketAddr {
    let server = Server::builder()
        .mtls(server_cert_pem, server_key_pem, client_ca_pem, mode)
        .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let greeter = GreeterServer::new(TestGreeter);
    let router = Router::new().add_service(GreeterServer::<TestGreeter>::NAME, greeter);

    tokio::spawn(async move {
        server.serve_with_listener(listener, router).await.unwrap();
    });

    wait_for_server(addr).await;
    addr
}

async fn connect_mtls(
    addr: SocketAddr,
    tls_config: rustls::ClientConfig,
) -> Result<GreeterClient<Channel>, grpc_core::BoxError> {
    let uri: http::Uri = format!("https://127.0.0.1:{}", addr.port())
        .parse()
        .unwrap();
    let channel = Channel::connect_with_tls_config(uri.clone(), tls_config).await?;
    Ok(GreeterClient::with_origin(channel, uri))
}

/// Connects and issues one `say_hello` call, returning the reply message.
///
/// `connect_with_tls_config` never opens a socket — the underlying hyper
/// client is lazy — so a failed TLS handshake (missing/untrusted client
/// cert) only ever surfaces on this first real RPC, not on connect itself.
async fn try_say_hello(
    addr: SocketAddr,
    tls_config: rustls::ClientConfig,
    name: &str,
) -> Result<String, grpc_core::BoxError> {
    let mut client = connect_mtls(addr, tls_config).await?;
    let resp = client
        .say_hello(HelloRequest { name: name.into() })
        .await
        .map_err(|e| -> grpc_core::BoxError { Box::new(e) })?;
    Ok(resp.get_ref().message.clone())
}

/// Everything a test needs to stand up a trusted-CA pair for the server
/// side, so each test only has to think about the client cert it presents.
struct TestPki {
    server_cert: Vec<u8>,
    server_key: Vec<u8>,
    server_ca_pem: Vec<u8>,
    client_ca_cert: Certificate,
    client_ca_key: KeyPair,
    client_ca_pem: Vec<u8>,
}

fn setup_pki() -> TestPki {
    let (server_ca_cert, server_ca_key, server_ca_pem) = generate_ca("Server CA");
    let (server_cert, server_key, _) = generate_leaf(
        &server_ca_cert,
        &server_ca_key,
        "localhost",
        vec!["localhost".to_string(), "127.0.0.1".to_string()],
    );
    let (client_ca_cert, client_ca_key, client_ca_pem) = generate_ca("Client CA");

    TestPki {
        server_cert,
        server_key,
        server_ca_pem,
        client_ca_cert,
        client_ca_key,
        client_ca_pem,
    }
}

// --- Tests ---

#[tokio::test]
async fn unary_roundtrip_over_mtls_carries_verified_identity() {
    let pki = setup_pki();
    let (client_cert, client_key, client_cert_der) =
        generate_leaf(&pki.client_ca_cert, &pki.client_ca_key, "device-1", vec![]);

    let addr = start_mtls_server(
        &pki.server_cert,
        &pki.server_key,
        &pki.client_ca_pem,
        ClientAuth::Required,
    )
    .await;

    let tls_config = client_tls_config(&pki.server_ca_pem, Some((&client_cert, &client_key)));
    let message = try_say_hello(addr, tls_config, "test").await.unwrap();

    let expected_tag = format!("cert:{}", hex_encode(&client_cert_der));
    assert_eq!(message, format!("Hello, test! [{expected_tag}]"));
}

#[tokio::test]
async fn server_streaming_over_mtls_carries_verified_identity_on_every_message() {
    let pki = setup_pki();
    let (client_cert, client_key, client_cert_der) =
        generate_leaf(&pki.client_ca_cert, &pki.client_ca_key, "device-1", vec![]);

    let addr = start_mtls_server(
        &pki.server_cert,
        &pki.server_key,
        &pki.client_ca_pem,
        ClientAuth::Required,
    )
    .await;

    let tls_config = client_tls_config(&pki.server_ca_pem, Some((&client_cert, &client_key)));
    let mut client = connect_mtls(addr, tls_config).await.unwrap();

    let resp = client
        .say_hello_server_stream(HelloRequest {
            name: "stream".into(),
        })
        .await
        .unwrap();

    let mut stream = resp.into_inner();
    let mut messages = Vec::new();
    while let Some(reply) = stream.message().await.unwrap() {
        messages.push(reply.message);
    }

    let expected_tag = format!("cert:{}", hex_encode(&client_cert_der));
    assert_eq!(
        messages,
        vec![
            format!("stream-0 [{expected_tag}]"),
            format!("stream-1 [{expected_tag}]"),
        ]
    );
}

#[tokio::test]
async fn required_mode_rejects_connection_without_client_cert() {
    let pki = setup_pki();

    let addr = start_mtls_server(
        &pki.server_cert,
        &pki.server_key,
        &pki.client_ca_pem,
        ClientAuth::Required,
    )
    .await;

    let tls_config = client_tls_config(&pki.server_ca_pem, None);
    let result = try_say_hello(addr, tls_config, "no-cert").await;

    assert!(result.is_err());
}

#[tokio::test]
async fn required_mode_rejects_client_cert_from_untrusted_ca() {
    let pki = setup_pki();
    let (untrusted_ca_cert, untrusted_ca_key, _) = generate_ca("Untrusted CA");
    let (client_cert, client_key, _) =
        generate_leaf(&untrusted_ca_cert, &untrusted_ca_key, "intruder", vec![]);

    let addr = start_mtls_server(
        &pki.server_cert,
        &pki.server_key,
        &pki.client_ca_pem,
        ClientAuth::Required,
    )
    .await;

    let tls_config = client_tls_config(&pki.server_ca_pem, Some((&client_cert, &client_key)));
    let result = try_say_hello(addr, tls_config, "intruder").await;

    assert!(result.is_err());
}

#[tokio::test]
async fn optional_mode_allows_missing_client_cert_with_no_identity() {
    let pki = setup_pki();

    let addr = start_mtls_server(
        &pki.server_cert,
        &pki.server_key,
        &pki.client_ca_pem,
        ClientAuth::Optional,
    )
    .await;

    let tls_config = client_tls_config(&pki.server_ca_pem, None);
    let message = try_say_hello(addr, tls_config, "anon").await.unwrap();

    assert_eq!(message, "Hello, anon! [cert:none]");
}
