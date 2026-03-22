use grpc_core::body::Body;
use grpc_core::codec::compression::{
    CompressionEncoding, EnabledCompressionEncodings, ACCEPT_ENCODING_HEADER, ENCODING_HEADER,
};
use grpc_core::codec::{Codec, Decoder, EncodeBody, Streaming};
use grpc_core::metadata::GRPC_CONTENT_TYPE;
use grpc_core::{Code, Request, Response, Status};
use http::header::{HeaderValue, CONTENT_TYPE, TE};
use http::uri::{PathAndQuery, Uri};
use http_body::Body as HttpBody;
use std::pin::pin;
use std::task::{Context, Poll};
use std::{fmt, future};
use tokio_stream::{Stream, StreamExt};

/// Definition of the gRPC service trait for client transports.
///
/// Any `tower::Service<http::Request<ReqBody>>` returning `http::Response<ResBody>`
/// automatically implements this.
pub trait GrpcService<ReqBody> {
    type ResponseBody: HttpBody;
    type Error: Into<grpc_core::BoxError>;
    type Future: std::future::Future<
        Output = Result<http::Response<Self::ResponseBody>, Self::Error>,
    >;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>>;
    fn call(&mut self, request: http::Request<ReqBody>) -> Self::Future;
}

impl<T, ReqBody, ResBody> GrpcService<ReqBody> for T
where
    T: tower_service::Service<http::Request<ReqBody>, Response = http::Response<ResBody>>,
    T::Error: Into<grpc_core::BoxError>,
    ResBody: HttpBody,
    <ResBody as HttpBody>::Error: Into<grpc_core::BoxError>,
{
    type ResponseBody = ResBody;
    type Error = T::Error;
    type Future = T::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        tower_service::Service::poll_ready(self, cx)
    }

    fn call(&mut self, request: http::Request<ReqBody>) -> Self::Future {
        tower_service::Service::call(self, request)
    }
}

/// A gRPC client dispatcher.
///
/// Wraps an inner transport service and handles encoding/decoding of
/// gRPC messages for all four RPC patterns.
#[derive(Clone)]
pub struct Grpc<T> {
    inner: T,
    origin: Uri,
    accept_compression_encodings: EnabledCompressionEncodings,
    send_compression_encoding: Option<CompressionEncoding>,
    max_decoding_message_size: Option<usize>,
    max_encoding_message_size: Option<usize>,
}

impl<T> Grpc<T> {
    pub fn new(inner: T) -> Self {
        Self::with_origin(inner, Uri::default())
    }

    pub fn with_origin(inner: T, origin: Uri) -> Self {
        Self {
            inner,
            origin,
            accept_compression_encodings: EnabledCompressionEncodings::default(),
            send_compression_encoding: None,
            max_decoding_message_size: None,
            max_encoding_message_size: None,
        }
    }

    /// Compress requests with this encoding.
    pub fn send_compressed(mut self, encoding: CompressionEncoding) -> Self {
        self.send_compression_encoding = Some(encoding);
        self
    }

    /// Accept compressed responses with this encoding.
    pub fn accept_compressed(mut self, encoding: CompressionEncoding) -> Self {
        self.accept_compression_encodings.enable(encoding);
        self
    }

    pub fn max_decoding_message_size(mut self, limit: usize) -> Self {
        self.max_decoding_message_size = Some(limit);
        self
    }

    pub fn max_encoding_message_size(mut self, limit: usize) -> Self {
        self.max_encoding_message_size = Some(limit);
        self
    }

    /// Wait until the transport is ready to accept a request.
    pub async fn ready(&mut self) -> Result<(), T::Error>
    where
        T: GrpcService<Body>,
    {
        future::poll_fn(|cx| self.inner.poll_ready(cx)).await
    }

    /// Send a single unary gRPC request.
    pub async fn unary<M1, M2, C>(
        &mut self,
        request: Request<M1>,
        path: PathAndQuery,
        codec: C,
    ) -> Result<Response<M2>, Status>
    where
        T: GrpcService<Body>,
        T::ResponseBody: HttpBody + Send + 'static,
        <T::ResponseBody as HttpBody>::Error: Into<grpc_core::BoxError>,
        C: Codec<Encode = M1, Decode = M2>,
        M1: Send + Sync + 'static,
        M2: Send + Sync + 'static,
    {
        let request = request.map(|m| tokio_stream::once(m));
        self.client_streaming(request, path, codec).await
    }

    /// Send a client-streaming gRPC request.
    pub async fn client_streaming<S, M1, M2, C>(
        &mut self,
        request: Request<S>,
        path: PathAndQuery,
        codec: C,
    ) -> Result<Response<M2>, Status>
    where
        T: GrpcService<Body>,
        T::ResponseBody: HttpBody + Send + 'static,
        <T::ResponseBody as HttpBody>::Error: Into<grpc_core::BoxError>,
        S: Stream<Item = M1> + Send + 'static,
        C: Codec<Encode = M1, Decode = M2>,
        M1: Send + Sync + 'static,
        M2: Send + Sync + 'static,
    {
        let (mut parts, body, extensions) =
            self.streaming(request, path, codec).await?.into_parts();

        let mut body = pin!(body);

        let message = body
            .try_next()
            .await
            .map_err(|mut status| {
                status.metadata_mut().merge(parts.clone());
                status
            })?
            .ok_or_else(|| Status::internal("Missing response message."))?;

        if let Some(trailers) = body.trailers().await? {
            parts.merge(trailers);
        }

        Ok(Response::from_parts(parts, message, extensions))
    }

    /// Send a server-streaming gRPC request.
    pub async fn server_streaming<M1, M2, C>(
        &mut self,
        request: Request<M1>,
        path: PathAndQuery,
        codec: C,
    ) -> Result<Response<Streaming<M2>>, Status>
    where
        T: GrpcService<Body>,
        T::ResponseBody: HttpBody + Send + 'static,
        <T::ResponseBody as HttpBody>::Error: Into<grpc_core::BoxError>,
        C: Codec<Encode = M1, Decode = M2>,
        M1: Send + Sync + 'static,
        M2: Send + Sync + 'static,
    {
        let request = request.map(|m| tokio_stream::once(m));
        self.streaming(request, path, codec).await
    }

    /// Send a bidirectional streaming gRPC request.
    ///
    /// This is the core implementation — all other methods delegate here.
    pub async fn streaming<S, M1, M2, C>(
        &mut self,
        request: Request<S>,
        path: PathAndQuery,
        mut codec: C,
    ) -> Result<Response<Streaming<M2>>, Status>
    where
        T: GrpcService<Body>,
        T::ResponseBody: HttpBody + Send + 'static,
        <T::ResponseBody as HttpBody>::Error: Into<grpc_core::BoxError>,
        S: Stream<Item = M1> + Send + 'static,
        C: Codec<Encode = M1, Decode = M2>,
        M1: Send + Sync + 'static,
        M2: Send + Sync + 'static,
    {
        let request = request
            .map(|s| {
                EncodeBody::new_client(
                    codec.encoder(),
                    s.map(Ok),
                    self.send_compression_encoding,
                    self.max_encoding_message_size,
                )
            })
            .map(Body::new);

        let request = self.prepare_request(request, path)?;

        let response = self
            .inner
            .call(request)
            .await
            .map_err(|err| Status::from_error(err.into()))?;

        let decoder = codec.decoder();
        self.create_response(decoder, response)
    }

    fn prepare_request(
        &self,
        request: Request<Body>,
        path: PathAndQuery,
    ) -> Result<http::Request<Body>, Status> {
        let mut parts = self.origin.clone().into_parts();

        match &parts.path_and_query {
            Some(pnq) if pnq != "/" => {
                parts.path_and_query = Some(
                    format!("{}{}", pnq.path(), path)
                        .parse()
                        .map_err(|e| Status::internal(format!("invalid path_and_query: {e}")))?,
                );
            }
            _ => {
                parts.path_and_query = Some(path);
            }
        }

        let uri =
            Uri::from_parts(parts).map_err(|e| Status::internal(format!("invalid URI: {e}")))?;

        let mut request = request.into_http(
            uri,
            http::Method::POST,
            http::Version::HTTP_2,
            true, // sanitize headers
        );

        request
            .headers_mut()
            .insert(TE, HeaderValue::from_static("trailers"));
        request
            .headers_mut()
            .insert(CONTENT_TYPE, GRPC_CONTENT_TYPE);

        if let Some(encoding) = self.send_compression_encoding {
            request
                .headers_mut()
                .insert(ENCODING_HEADER, encoding.into_header_value());
        }

        if let Some(header_value) = self
            .accept_compression_encodings
            .into_accept_encoding_header_value()
        {
            request
                .headers_mut()
                .insert(ACCEPT_ENCODING_HEADER, header_value);
        }

        Ok(request)
    }

    fn create_response<M2>(
        &self,
        decoder: impl Decoder<Item = M2, Error = Status> + Send + 'static,
        response: http::Response<T::ResponseBody>,
    ) -> Result<Response<Streaming<M2>>, Status>
    where
        T: GrpcService<Body>,
        T::ResponseBody: HttpBody + Send + 'static,
        <T::ResponseBody as HttpBody>::Error: Into<grpc_core::BoxError>,
    {
        let encoding = CompressionEncoding::from_encoding_header(
            response.headers(),
            self.accept_compression_encodings,
        )?;

        let status_code = response.status();
        let trailers_only_status = Status::from_header_map(response.headers());

        let expect_additional_trailers = if let Some(status) = trailers_only_status {
            if status.code() != Code::Ok {
                return Err(status);
            }
            false
        } else {
            true
        };

        let response = response.map(|body| {
            if expect_additional_trailers {
                Streaming::new_response(
                    decoder,
                    body,
                    status_code,
                    encoding,
                    self.max_decoding_message_size,
                )
            } else {
                Streaming::new_empty(decoder, body)
            }
        });

        Ok(Response::from_http(response))
    }
}

impl<T: fmt::Debug> fmt::Debug for Grpc<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Grpc")
            .field("inner", &self.inner)
            .field("origin", &self.origin)
            .field(
                "accept_compression_encodings",
                &self.accept_compression_encodings,
            )
            .field("send_compression_encoding", &self.send_compression_encoding)
            .field("max_decoding_message_size", &self.max_decoding_message_size)
            .field("max_encoding_message_size", &self.max_encoding_message_size)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grpc_new_default_origin() {
        let grpc = Grpc::new(());
        assert_eq!(grpc.origin, Uri::default());
    }

    #[test]
    fn grpc_with_origin() {
        let grpc = Grpc::with_origin((), "http://localhost:50051".parse().unwrap());
        assert_eq!(grpc.origin.host(), Some("localhost"));
    }

    #[test]
    fn grpc_clone() {
        let grpc = Grpc::new(42u32);
        let grpc2 = grpc.clone();
        assert_eq!(grpc2.inner, 42);
    }

    #[test]
    fn prepare_request_sets_headers() {
        let grpc = Grpc::new(());
        let req = Request::new(Body::empty());
        let path: PathAndQuery = "/test.Svc/Method".parse().unwrap();

        let http_req = grpc.prepare_request(req, path).unwrap();

        assert_eq!(http_req.method(), http::Method::POST);
        assert_eq!(http_req.uri().path(), "/test.Svc/Method");
        assert_eq!(
            http_req.headers().get("content-type").unwrap(),
            "application/grpc"
        );
        assert_eq!(http_req.headers().get("te").unwrap(), "trailers");
    }

    #[test]
    fn prepare_request_with_origin_path() {
        let grpc = Grpc::with_origin((), "http://proxy:8080/prefix".parse().unwrap());
        let req = Request::new(Body::empty());
        let path: PathAndQuery = "/test.Svc/Method".parse().unwrap();

        let http_req = grpc.prepare_request(req, path).unwrap();
        assert_eq!(http_req.uri().path(), "/prefix/test.Svc/Method");
    }

    #[cfg(feature = "gzip")]
    #[test]
    fn prepare_request_with_compression() {
        use grpc_core::codec::compression::CompressionEncoding;

        let grpc = Grpc::new(())
            .send_compressed(CompressionEncoding::Gzip)
            .accept_compressed(CompressionEncoding::Gzip);

        let req = Request::new(Body::empty());
        let path: PathAndQuery = "/test.Svc/Method".parse().unwrap();

        let http_req = grpc.prepare_request(req, path).unwrap();

        assert_eq!(http_req.headers().get("grpc-encoding").unwrap(), "gzip");
        assert_eq!(
            http_req.headers().get("grpc-accept-encoding").unwrap(),
            "gzip"
        );
    }

    #[test]
    fn builder_methods() {
        let grpc = Grpc::new(())
            .max_decoding_message_size(1024)
            .max_encoding_message_size(2048);
        assert_eq!(grpc.max_decoding_message_size, Some(1024));
        assert_eq!(grpc.max_encoding_message_size, Some(2048));
    }

    // --- Mock transport for client Grpc tests ---

    use bytes::{BufMut, BytesMut};
    use grpc_core::codec::{BufferSettings, DecodeBuf, Decoder, EncodeBuf, Encoder};

    #[derive(Debug, Default, Clone)]
    struct TestCodec;

    #[derive(Debug, Default)]
    struct TestEncoder;

    #[derive(Debug, Default)]
    struct TestDecoder;

    impl grpc_core::codec::Codec for TestCodec {
        type Encode = Vec<u8>;
        type Decode = Vec<u8>;
        type Encoder = TestEncoder;
        type Decoder = TestDecoder;

        fn encoder(&mut self) -> Self::Encoder {
            TestEncoder
        }

        fn decoder(&mut self) -> Self::Decoder {
            TestDecoder
        }
    }

    impl Encoder for TestEncoder {
        type Item = Vec<u8>;
        type Error = Status;

        fn encode(&mut self, item: Self::Item, buf: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
            buf.put_slice(&item);
            Ok(())
        }
    }

    impl Decoder for TestDecoder {
        type Item = Vec<u8>;
        type Error = Status;

        fn decode(&mut self, buf: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
            use bytes::Buf;
            let len = buf.remaining();
            if len == 0 {
                return Ok(None);
            }
            let data = buf.copy_to_bytes(len).to_vec();
            Ok(Some(data))
        }

        fn buffer_settings(&self) -> BufferSettings {
            BufferSettings::default()
        }
    }

    /// Build a gRPC-framed response body (for mock transport)
    fn build_grpc_response_body(payload: &[u8]) -> bytes::Bytes {
        let mut buf = BytesMut::new();
        buf.put_u8(0); // no compression
        buf.put_u32(payload.len() as u32);
        buf.put_slice(payload);
        buf.freeze()
    }

    /// A mock transport that echoes the payload back in a proper gRPC response
    #[derive(Clone)]
    struct MockEchoTransport;

    impl tower_service::Service<http::Request<Body>> for MockEchoTransport {
        type Response = http::Response<Body>;
        type Error = Box<dyn std::error::Error + Send + Sync>;
        type Future = std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
        >;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, req: http::Request<Body>) -> Self::Future {
            Box::pin(async move {
                // Read the request body to get the payload
                let body = req.into_body();
                let collected = http_body_util::BodyExt::collect(body).await.unwrap();
                let data = collected.to_bytes();

                // Extract the payload from the gRPC frame
                let payload = if data.len() >= 5 { &data[5..] } else { &[] };

                let response_body = build_grpc_response_body(payload);

                // Build response with trailers indicating OK status
                let resp = http::Response::builder()
                    .status(200)
                    .header("content-type", "application/grpc")
                    .body(Body::new(http_body_util::Full::new(response_body)))
                    .unwrap();

                Ok(resp)
            })
        }
    }

    // CL7: Grpc::ready() — verifies the transport poll_ready is called
    #[tokio::test]
    async fn grpc_ready_succeeds() {
        let mut grpc = Grpc::new(MockEchoTransport);
        // Should succeed because MockEchoTransport always returns Poll::Ready(Ok(()))
        grpc.ready().await.unwrap();
    }

    // CL8: Grpc::unary() — core unary RPC pattern
    #[tokio::test]
    async fn grpc_unary_roundtrip() {
        let mut grpc = Grpc::new(MockEchoTransport);
        let req = Request::new(b"hello".to_vec());
        let path: PathAndQuery = "/test.Svc/Echo".parse().unwrap();

        let resp = grpc.unary(req, path, TestCodec).await.unwrap();
        assert_eq!(resp.into_inner(), b"hello");
    }

    // CL10: Grpc::server_streaming() — server streaming RPC pattern
    #[tokio::test]
    async fn grpc_server_streaming() {
        let mut grpc = Grpc::new(MockEchoTransport);
        let req = Request::new(b"stream-test".to_vec());
        let path: PathAndQuery = "/test.Svc/StreamMethod".parse().unwrap();

        let resp = grpc.server_streaming(req, path, TestCodec).await.unwrap();
        let mut stream = resp.into_inner();
        let msg = stream.message().await.unwrap().unwrap();
        assert_eq!(msg, b"stream-test");
    }

    // CL12: prepare_request path merging — origin + path concatenation
    #[test]
    fn prepare_request_root_origin_path() {
        let grpc = Grpc::with_origin((), "http://localhost:50051/".parse().unwrap());
        let req = Request::new(Body::empty());
        let path: PathAndQuery = "/test.Svc/Method".parse().unwrap();

        let http_req = grpc.prepare_request(req, path).unwrap();
        // Root path "/" should just use the given path
        assert_eq!(http_req.uri().path(), "/test.Svc/Method");
    }

    // CL13: create_response — trailers-only error status
    #[tokio::test]
    async fn create_response_trailers_only_error() {
        let grpc = Grpc::new(MockEchoTransport);

        // Build a trailers-only response with error status
        let resp = http::Response::builder()
            .status(200)
            .header("content-type", "application/grpc")
            .header("grpc-status", "7") // PermissionDenied
            .header("grpc-message", "denied")
            .body(Body::new(http_body_util::Full::new(bytes::Bytes::new())))
            .unwrap();

        let result = grpc.create_response(TestDecoder, resp);
        assert!(result.is_err());
        let status = result.unwrap_err();
        assert_eq!(status.code(), Code::PermissionDenied);
    }

    // CL13: create_response — trailers-only OK status
    #[tokio::test]
    async fn create_response_trailers_only_ok() {
        let grpc = Grpc::new(MockEchoTransport);

        let resp = http::Response::builder()
            .status(200)
            .header("content-type", "application/grpc")
            .header("grpc-status", "0") // Ok
            .body(Body::new(http_body_util::Full::new(bytes::Bytes::new())))
            .unwrap();

        let result = grpc.create_response(TestDecoder, resp);
        assert!(result.is_ok());
    }

    // CL13: create_response — normal response (no trailers-only)
    #[tokio::test]
    async fn create_response_normal_response() {
        let grpc = Grpc::new(MockEchoTransport);

        let body = build_grpc_response_body(b"test-data");
        let resp = http::Response::builder()
            .status(200)
            .header("content-type", "application/grpc")
            .body(Body::new(http_body_util::Full::new(body)))
            .unwrap();

        let result = grpc.create_response(TestDecoder, resp);
        assert!(result.is_ok());
    }

    #[test]
    fn grpc_debug_impl() {
        let grpc = Grpc::new(42u32);
        let debug = format!("{grpc:?}");
        assert!(debug.contains("Grpc"));
        assert!(debug.contains("42"));
    }
}
