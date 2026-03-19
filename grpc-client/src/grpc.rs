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
}
