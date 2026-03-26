use crate::service::{
    ClientStreamingService, ServerStreamingService, StreamingService, UnaryService,
};
use grpc_core::body::Body;
use grpc_core::codec::compression::{
    CompressionEncoding, EnabledCompressionEncodings, ENCODING_HEADER,
};
use grpc_core::codec::{Codec, EncodeBody, Streaming};
use grpc_core::metadata::GRPC_CONTENT_TYPE;
use grpc_core::{Request, Status};
use http_body::Body as HttpBody;
use std::fmt;
use std::pin::pin;
use tokio_stream::{Stream, StreamExt};

macro_rules! t {
    ($result:expr) => {
        match $result {
            Ok(value) => value,
            Err(status) => return status.into_http(),
        }
    };
}

/// A gRPC server handler.
///
/// Dispatches incoming HTTP/2 requests to the appropriate service method
/// for each of the four RPC patterns (unary, server-streaming,
/// client-streaming, bidirectional).
pub struct Grpc<T> {
    codec: T,
    accept_compression_encodings: EnabledCompressionEncodings,
    send_compression_encodings: EnabledCompressionEncodings,
    max_decoding_message_size: Option<usize>,
    max_encoding_message_size: Option<usize>,
}

impl<T> Grpc<T>
where
    T: Codec,
{
    pub fn new(codec: T) -> Self {
        Self {
            codec,
            accept_compression_encodings: EnabledCompressionEncodings::default(),
            send_compression_encodings: EnabledCompressionEncodings::default(),
            max_decoding_message_size: None,
            max_encoding_message_size: None,
        }
    }

    /// Enable accepting compressed requests with this encoding.
    pub fn accept_compressed(mut self, encoding: CompressionEncoding) -> Self {
        self.accept_compression_encodings.enable(encoding);
        self
    }

    /// Enable sending compressed responses with this encoding.
    pub fn send_compressed(mut self, encoding: CompressionEncoding) -> Self {
        self.send_compression_encodings.enable(encoding);
        self
    }

    pub fn max_decoding_message_size(mut self, limit: usize) -> Self {
        assert!(limit > 0, "max message size must be > 0");
        self.max_decoding_message_size = Some(limit);
        self
    }

    pub fn max_encoding_message_size(mut self, limit: usize) -> Self {
        assert!(limit > 0, "max message size must be > 0");
        self.max_encoding_message_size = Some(limit);
        self
    }

    /// Handle a single unary gRPC request.
    pub async fn unary<S, B>(
        &mut self,
        mut service: S,
        req: http::Request<B>,
    ) -> http::Response<Body>
    where
        S: UnaryService<T::Decode, Response = T::Encode>,
        B: HttpBody + Send + 'static,
        B::Error: Into<grpc_core::BoxError> + Send,
    {
        let accept_encoding = CompressionEncoding::from_accept_encoding_header(
            req.headers(),
            self.send_compression_encodings,
        );

        let request = match self.map_request_unary(req).await {
            Ok(r) => r,
            Err(status) => return status.into_http(),
        };

        let response = service
            .call(request)
            .await
            .map(|r| r.map(|m| tokio_stream::once(Ok(m))));

        self.map_response(response, accept_encoding)
    }

    /// Handle a server-side streaming request.
    pub async fn server_streaming<S, B>(
        &mut self,
        mut service: S,
        req: http::Request<B>,
    ) -> http::Response<Body>
    where
        S: ServerStreamingService<T::Decode, Response = T::Encode>,
        S::ResponseStream: Send + 'static,
        B: HttpBody + Send + 'static,
        B::Error: Into<grpc_core::BoxError> + Send,
    {
        let accept_encoding = CompressionEncoding::from_accept_encoding_header(
            req.headers(),
            self.send_compression_encodings,
        );

        let request = match self.map_request_unary(req).await {
            Ok(r) => r,
            Err(status) => return status.into_http(),
        };

        let response = service.call(request).await;
        self.map_response(response, accept_encoding)
    }

    /// Handle a client-side streaming request.
    pub async fn client_streaming<S, B>(
        &mut self,
        mut service: S,
        req: http::Request<B>,
    ) -> http::Response<Body>
    where
        S: ClientStreamingService<T::Decode, Response = T::Encode>,
        B: HttpBody + Send + 'static,
        B::Error: Into<grpc_core::BoxError> + Send + 'static,
    {
        let accept_encoding = CompressionEncoding::from_accept_encoding_header(
            req.headers(),
            self.send_compression_encodings,
        );
        let request = t!(self.map_request_streaming(req));

        let response = service
            .call(request)
            .await
            .map(|r| r.map(|m| tokio_stream::once(Ok(m))));

        self.map_response(response, accept_encoding)
    }

    /// Handle a bidirectional streaming request.
    pub async fn streaming<S, B>(
        &mut self,
        mut service: S,
        req: http::Request<B>,
    ) -> http::Response<Body>
    where
        S: StreamingService<T::Decode, Response = T::Encode> + Send,
        S::ResponseStream: Send + 'static,
        B: HttpBody + Send + 'static,
        B::Error: Into<grpc_core::BoxError> + Send,
    {
        let accept_encoding = CompressionEncoding::from_accept_encoding_header(
            req.headers(),
            self.send_compression_encodings,
        );
        let request = t!(self.map_request_streaming(req));
        let response = service.call(request).await;
        self.map_response(response, accept_encoding)
    }

    /// Decode a single message from the request body (for unary + server-streaming).
    async fn map_request_unary<B>(
        &mut self,
        request: http::Request<B>,
    ) -> Result<Request<T::Decode>, Status>
    where
        B: HttpBody + Send + 'static,
        B::Error: Into<grpc_core::BoxError> + Send,
    {
        let request_encoding = CompressionEncoding::from_encoding_header(
            request.headers(),
            self.accept_compression_encodings,
        )?;

        let (parts, body) = request.into_parts();

        let mut stream = pin!(Streaming::new_request(
            self.codec.decoder(),
            body,
            request_encoding,
            self.max_decoding_message_size,
        ));

        let message = stream
            .try_next()
            .await?
            .ok_or_else(|| Status::internal("Missing request message."))?;

        let mut req = Request::from_http_parts(parts, message);

        if let Some(trailers) = stream.trailers().await? {
            req.metadata_mut().merge(trailers);
        }

        Ok(req)
    }

    /// Wrap the request body as a Streaming decoder (for client-streaming + bidi).
    fn map_request_streaming<B>(
        &mut self,
        request: http::Request<B>,
    ) -> Result<Request<Streaming<T::Decode>>, Status>
    where
        B: HttpBody + Send + 'static,
        B::Error: Into<grpc_core::BoxError> + Send,
    {
        let encoding = CompressionEncoding::from_encoding_header(
            request.headers(),
            self.accept_compression_encodings,
        )?;

        let request = request.map(|body| {
            Streaming::new_request(
                self.codec.decoder(),
                body,
                encoding,
                self.max_decoding_message_size,
            )
        });

        Ok(Request::from_http(request))
    }

    /// Encode the response stream into an HTTP response with gRPC framing.
    fn map_response<B>(
        &mut self,
        response: Result<grpc_core::Response<B>, Status>,
        accept_encoding: Option<CompressionEncoding>,
    ) -> http::Response<Body>
    where
        B: Stream<Item = Result<T::Encode, Status>> + Send + 'static,
    {
        let response = t!(response);

        let (mut parts, body) = response.into_http().into_parts();

        parts
            .headers
            .insert(http::header::CONTENT_TYPE, GRPC_CONTENT_TYPE);

        if let Some(encoding) = accept_encoding {
            parts
                .headers
                .insert(ENCODING_HEADER, encoding.into_header_value());
        }

        let body = EncodeBody::new_server(
            self.codec.encoder(),
            body,
            accept_encoding,
            self.max_encoding_message_size,
        );

        http::Response::from_parts(parts, Body::new(body))
    }
}

impl<T: fmt::Debug> fmt::Debug for Grpc<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Grpc").field("codec", &self.codec).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{BufMut, BytesMut};
    use grpc_core::codec::test_helpers::TestCodec;

    // --- Test service ---

    struct EchoService;

    impl UnaryService<Vec<u8>> for EchoService {
        type Response = Vec<u8>;
        type Future = std::future::Ready<Result<grpc_core::Response<Vec<u8>>, Status>>;

        fn call(&mut self, request: Request<Vec<u8>>) -> Self::Future {
            let msg = request.into_inner();
            std::future::ready(Ok(grpc_core::Response::new(msg)))
        }
    }

    fn build_grpc_request(payload: &[u8]) -> http::Request<http_body_util::Full<bytes::Bytes>> {
        let mut buf = BytesMut::new();
        buf.put_u8(0); // no compression
        buf.put_u32(payload.len() as u32);
        buf.put_slice(payload);

        http::Request::builder()
            .method(http::Method::POST)
            .uri("/test.TestService/Echo")
            .header("content-type", "application/grpc")
            .body(http_body_util::Full::new(buf.freeze()))
            .unwrap()
    }

    #[tokio::test]
    async fn unary_echo_roundtrip() {
        let mut grpc = Grpc::new(TestCodec);
        let req = build_grpc_request(b"hello");

        let response = grpc.unary(EchoService, req).await;

        assert_eq!(response.status(), http::StatusCode::OK);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/grpc"
        );

        // Verify the response body actually contains the echoed message.
        let body = response.into_body();
        let collected = http_body_util::BodyExt::collect(body).await.unwrap();
        let data = collected.to_bytes();
        // gRPC frame: 1 byte compression flag + 4 byte length + payload
        assert!(data.len() >= 5, "response body too short: {}", data.len());
        let payload = &data[5..];
        assert_eq!(payload, b"hello");
    }

    #[test]
    fn grpc_debug_impl() {
        let grpc = Grpc::new(TestCodec);
        let debug = format!("{grpc:?}");
        assert!(debug.contains("Grpc"));
    }

    #[cfg(feature = "gzip")]
    fn build_gzip_grpc_request(
        payload: &[u8],
    ) -> http::Request<http_body_util::Full<bytes::Bytes>> {
        use flate2::write::GzEncoder;
        use std::io::Write;

        let mut encoder = GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(payload).unwrap();
        let compressed = encoder.finish().unwrap();

        let mut buf = BytesMut::new();
        buf.put_u8(1); // compressed
        buf.put_u32(compressed.len() as u32);
        buf.put_slice(&compressed);

        http::Request::builder()
            .method(http::Method::POST)
            .uri("/test.TestService/Echo")
            .header("content-type", "application/grpc")
            .header("grpc-encoding", "gzip")
            .header("grpc-accept-encoding", "gzip")
            .body(http_body_util::Full::new(buf.freeze()))
            .unwrap()
    }

    #[cfg(feature = "gzip")]
    #[tokio::test]
    async fn unary_gzip_compressed_request() {
        use grpc_core::codec::compression::CompressionEncoding;

        let mut grpc = Grpc::new(TestCodec)
            .accept_compressed(CompressionEncoding::Gzip)
            .send_compressed(CompressionEncoding::Gzip);

        let req = build_gzip_grpc_request(b"compressed-hello");
        let response = grpc.unary(EchoService, req).await;

        assert_eq!(response.status(), http::StatusCode::OK);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/grpc"
        );
        // Server should indicate gzip encoding in response
        assert_eq!(response.headers().get("grpc-encoding").unwrap(), "gzip");
    }

    // S7: Unsupported compression encoding in request — server should reject
    #[cfg(feature = "gzip")]
    #[tokio::test]
    async fn unsupported_encoding_in_request_rejected() {
        // Server does NOT accept gzip — only default (none)
        let mut grpc = Grpc::new(TestCodec);

        let req = http::Request::builder()
            .method(http::Method::POST)
            .uri("/test.TestService/Echo")
            .header("content-type", "application/grpc")
            .header("grpc-encoding", "gzip")
            .body(http_body_util::Full::new(bytes::Bytes::new()))
            .unwrap();

        let response = grpc.unary(EchoService, req).await;
        // Should be a gRPC error response (Unimplemented)
        let status = response.headers().get("grpc-status").unwrap();
        assert_eq!(status, "12"); // Code::Unimplemented
    }

    // S8: Empty request message — stream yields None
    #[tokio::test]
    async fn empty_request_body_returns_internal_error() {
        let mut grpc = Grpc::new(TestCodec);

        // Build a request with an empty body (no gRPC frame at all)
        let req = http::Request::builder()
            .method(http::Method::POST)
            .uri("/test.TestService/Echo")
            .header("content-type", "application/grpc")
            .body(http_body_util::Full::new(bytes::Bytes::new()))
            .unwrap();

        let response = grpc.unary(EchoService, req).await;
        // Should get Internal error because stream yields None ("Missing request message.")
        let status = response.headers().get("grpc-status").unwrap();
        assert_eq!(status, "13"); // Code::Internal
    }

    // S9: Service error response — the t! macro Err path in map_response
    struct FailingService;

    impl UnaryService<Vec<u8>> for FailingService {
        type Response = Vec<u8>;
        type Future = std::future::Ready<Result<grpc_core::Response<Vec<u8>>, Status>>;

        fn call(&mut self, _request: Request<Vec<u8>>) -> Self::Future {
            std::future::ready(Err(Status::permission_denied("not allowed")))
        }
    }

    #[tokio::test]
    async fn service_error_response_maps_to_grpc_status() {
        let mut grpc = Grpc::new(TestCodec);
        let req = build_grpc_request(b"hello");

        let response = grpc.unary(FailingService, req).await;
        let status = response.headers().get("grpc-status").unwrap();
        assert_eq!(status, "7"); // Code::PermissionDenied
    }

    // S10: server_streaming pattern
    struct EchoServerStreamingService;

    impl ServerStreamingService<Vec<u8>> for EchoServerStreamingService {
        type Response = Vec<u8>;
        type ResponseStream = std::pin::Pin<Box<dyn Stream<Item = Result<Vec<u8>, Status>> + Send>>;
        type Future = std::future::Ready<Result<grpc_core::Response<Self::ResponseStream>, Status>>;

        fn call(&mut self, request: Request<Vec<u8>>) -> Self::Future {
            let msg = request.into_inner();
            let stream: Self::ResponseStream =
                Box::pin(tokio_stream::iter(vec![Ok(msg.clone()), Ok(msg)]));
            std::future::ready(Ok(grpc_core::Response::new(stream)))
        }
    }

    #[tokio::test]
    async fn server_streaming_roundtrip() {
        let mut grpc = Grpc::new(TestCodec);
        let req = build_grpc_request(b"hello");

        let response = grpc.server_streaming(EchoServerStreamingService, req).await;
        assert_eq!(response.status(), http::StatusCode::OK);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/grpc"
        );
    }

    // S11: client_streaming pattern
    struct SumClientStreamingService;

    impl ClientStreamingService<Vec<u8>> for SumClientStreamingService {
        type Response = Vec<u8>;
        type Future = std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<grpc_core::Response<Vec<u8>>, Status>>
                    + Send,
            >,
        >;

        fn call(&mut self, request: Request<grpc_core::Streaming<Vec<u8>>>) -> Self::Future {
            Box::pin(async move {
                let mut stream = request.into_inner();
                let mut total = 0usize;
                while let Some(msg) = stream.message().await? {
                    total += msg.len();
                }
                Ok(grpc_core::Response::new(total.to_string().into_bytes()))
            })
        }
    }

    #[tokio::test]
    async fn client_streaming_roundtrip() {
        let mut grpc = Grpc::new(TestCodec);
        let req = build_grpc_request(b"hello");

        let response = grpc.client_streaming(SumClientStreamingService, req).await;
        assert_eq!(response.status(), http::StatusCode::OK);
    }

    // S12: bidi streaming pattern
    struct EchoBidiService;

    impl StreamingService<Vec<u8>> for EchoBidiService {
        type Response = Vec<u8>;
        type ResponseStream = std::pin::Pin<Box<dyn Stream<Item = Result<Vec<u8>, Status>> + Send>>;
        type Future = std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<grpc_core::Response<Self::ResponseStream>, Status>,
                    > + Send,
            >,
        >;

        fn call(&mut self, request: Request<grpc_core::Streaming<Vec<u8>>>) -> Self::Future {
            Box::pin(async move {
                let mut stream = request.into_inner();
                let mut collected = Vec::new();
                while let Some(msg) = stream.message().await? {
                    collected.push(Ok(msg));
                }
                let response_stream: Self::ResponseStream = Box::pin(tokio_stream::iter(collected));
                Ok(grpc_core::Response::new(response_stream))
            })
        }
    }

    #[tokio::test]
    async fn bidi_streaming_roundtrip() {
        let mut grpc = Grpc::new(TestCodec);
        let req = build_grpc_request(b"hello");

        let response = grpc.streaming(EchoBidiService, req).await;
        assert_eq!(response.status(), http::StatusCode::OK);
    }

    // S13: Size limit enforcement
    #[test]
    #[should_panic(expected = "max message size must be > 0")]
    fn max_decoding_message_size_zero_panics() {
        Grpc::new(TestCodec).max_decoding_message_size(0);
    }

    #[test]
    #[should_panic(expected = "max message size must be > 0")]
    fn max_encoding_message_size_zero_panics() {
        Grpc::new(TestCodec).max_encoding_message_size(0);
    }
}
