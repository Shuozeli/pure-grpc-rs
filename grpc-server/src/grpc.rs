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
        self.max_decoding_message_size = Some(limit);
        self
    }

    pub fn max_encoding_message_size(mut self, limit: usize) -> Self {
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
            Err(status) => {
                return self.map_response::<tokio_stream::Once<Result<T::Encode, Status>>>(
                    Err(status),
                    accept_encoding,
                );
            }
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
            Err(status) => {
                return self.map_response::<S::ResponseStream>(Err(status), accept_encoding);
            }
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
    use bytes::{Buf, BufMut, BytesMut};
    use grpc_core::codec::{BufferSettings, DecodeBuf, Decoder, EncodeBuf, Encoder};

    // --- Test codec that encodes/decodes Vec<u8> ---

    #[derive(Debug, Default)]
    struct TestCodec;

    #[derive(Debug, Default)]
    struct TestEncoder;

    #[derive(Debug, Default)]
    struct TestDecoder;

    impl Codec for TestCodec {
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
            let len = buf.remaining();
            let data = buf.copy_to_bytes(len).to_vec();
            Ok(Some(data))
        }

        fn buffer_settings(&self) -> BufferSettings {
            BufferSettings::default()
        }
    }

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
    }

    #[test]
    fn grpc_debug_impl() {
        let grpc = Grpc::new(TestCodec);
        let debug = format!("{grpc:?}");
        assert!(debug.contains("Grpc"));
    }
}
