//! Tonic client for benchmarking

use crate::{BenchmarkConfig, STREAM_COUNT};
use proto::benchmark_service_client::BenchmarkServiceClient;

pub mod proto {
    tonic::include_proto!("benchmark");
}

pub struct TonicClient {
    client: BenchmarkServiceClient<tonic::transport::Channel>,
}

impl TonicClient {
    pub async fn connect(addr: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let channel = tonic::transport::Channel::from_static(addr)
            .connect()
            .await?;
        let client = BenchmarkServiceClient::new(channel);
        Ok(Self { client })
    }

    pub async fn unary_call(&mut self, id: u64, payload: Vec<u8>, numbers: Vec<u64>) -> Result<u64, Box<dyn std::error::Error>> {
        let request = proto::BenchmarkRequest {
            id,
            payload: String::from_utf8(payload).unwrap_or_default(),
            numbers,
        };

        let response = self.client.unary_call(request).await?;
        Ok(response.into_inner().id)
    }

    pub async fn server_stream(&mut self, id: u64, payload: String, numbers: Vec<u64>) -> Result<usize, Box<dyn std::error::Error>> {
        let request = proto::BenchmarkRequest { id, payload, numbers };
        let mut stream = self.client.server_stream(request).await?.into_inner();

        let mut count = 0;
        while let Some(_response) = stream.message().await? {
            count += 1;
        }
        Ok(count)
    }

    pub async fn client_stream(&mut self, count: usize) -> Result<u64, Box<dyn std::error::Error>> {
        use tokio_stream::StreamExt;

        let input_stream = tokio_stream::iter(0..count).map(|i| proto::BenchmarkRequest {
            id: i as u64,
            payload: "payload".to_string(),
            numbers: vec![1, 2, 3],
        });

        let request = tonic::Request::new(input_stream);
        let response = self.client.client_stream(request).await?;
        Ok(response.into_inner().id)
    }

    pub async fn bidi_stream(&mut self, count: usize) -> Result<usize, Box<dyn std::error::Error>> {
        use tokio_stream::StreamExt;

        let input_stream = tokio_stream::iter(0..count).map(|i| proto::BenchmarkRequest {
            id: i as u64,
            payload: "payload".to_string(),
            numbers: vec![1, 2, 3],
        });

        let request = tonic::Request::new(input_stream);
        let mut stream = self.client.bi_di_stream(request).await?.into_inner();

        let mut response_count = 0;
        while let Some(_response) = stream.message().await? {
            response_count += 1;
        }
        Ok(response_count)
    }
}
