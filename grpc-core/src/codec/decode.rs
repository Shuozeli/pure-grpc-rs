use super::compression::CompressionEncoding;
use super::{DecodeBuf, Decoder, DEFAULT_MAX_RECV_MESSAGE_SIZE, HEADER_SIZE};
use crate::{body::Body, metadata::MetadataMap, Code, Status};
use bytes::{Buf, BufMut, BytesMut};
use http::{HeaderMap, StatusCode};
use http_body::Body as HttpBody;
use http_body_util::BodyExt;
use std::{
    fmt, future,
    pin::Pin,
    task::{ready, Context, Poll},
};
use tokio_stream::Stream;
use tracing::trace;

/// Streaming requests and responses.
///
/// Wraps an HTTP body and a decoder, yielding decoded messages as a `Stream`.
pub struct Streaming<T> {
    decoder: Box<dyn Decoder<Item = T, Error = Status> + Send + 'static>,
    inner: StreamingInner,
}

struct StreamingInner {
    body: Body,
    state: State,
    direction: Direction,
    buf: BytesMut,
    trailers: Option<HeaderMap>,
    decompress_buf: BytesMut,
    encoding: Option<CompressionEncoding>,
    max_message_size: Option<usize>,
}

impl<T> Unpin for Streaming<T> {}

#[derive(Debug, Clone)]
enum State {
    ReadHeader,
    ReadBody {
        compression: Option<CompressionEncoding>,
        len: usize,
    },
    Error(Option<Status>),
}

#[derive(Debug, PartialEq, Eq)]
enum Direction {
    Request,
    Response(StatusCode),
    EmptyResponse,
}

impl<T> Streaming<T> {
    pub fn new_response<B, D>(
        decoder: D,
        body: B,
        status_code: StatusCode,
        encoding: Option<CompressionEncoding>,
        max_message_size: Option<usize>,
    ) -> Self
    where
        B: HttpBody + Send + 'static,
        B::Error: Into<crate::BoxError>,
        D: Decoder<Item = T, Error = Status> + Send + 'static,
    {
        Self::new(
            decoder,
            body,
            Direction::Response(status_code),
            encoding,
            max_message_size,
        )
    }

    pub fn new_empty<B, D>(decoder: D, body: B) -> Self
    where
        B: HttpBody + Send + 'static,
        B::Error: Into<crate::BoxError>,
        D: Decoder<Item = T, Error = Status> + Send + 'static,
    {
        Self::new(decoder, body, Direction::EmptyResponse, None, None)
    }

    pub fn new_request<B, D>(
        decoder: D,
        body: B,
        encoding: Option<CompressionEncoding>,
        max_message_size: Option<usize>,
    ) -> Self
    where
        B: HttpBody + Send + 'static,
        B::Error: Into<crate::BoxError>,
        D: Decoder<Item = T, Error = Status> + Send + 'static,
    {
        Self::new(
            decoder,
            body,
            Direction::Request,
            encoding,
            max_message_size,
        )
    }

    fn new<B, D>(
        decoder: D,
        body: B,
        direction: Direction,
        encoding: Option<CompressionEncoding>,
        max_message_size: Option<usize>,
    ) -> Self
    where
        B: HttpBody + Send + 'static,
        B::Error: Into<crate::BoxError>,
        D: Decoder<Item = T, Error = Status> + Send + 'static,
    {
        let buffer_size = decoder.buffer_settings().buffer_size;
        Self {
            decoder: Box::new(decoder),
            inner: StreamingInner {
                body: Body::new(
                    body.map_frame(|frame| {
                        frame.map_data(|mut buf| buf.copy_to_bytes(buf.remaining()))
                    })
                    .map_err(|err| Status::map_error(err.into())),
                ),
                state: State::ReadHeader,
                direction,
                buf: BytesMut::with_capacity(buffer_size),
                trailers: None,
                decompress_buf: BytesMut::new(),
                encoding,
                max_message_size,
            },
        }
    }

    /// Fetch the next message from this stream.
    pub async fn message(&mut self) -> Result<Option<T>, Status> {
        match future::poll_fn(|cx| Pin::new(&mut *self).poll_next(cx)).await {
            Some(Ok(m)) => Ok(Some(m)),
            Some(Err(e)) => Err(e),
            None => Ok(None),
        }
    }

    /// Fetch the trailing metadata.
    pub async fn trailers(&mut self) -> Result<Option<MetadataMap>, Status> {
        if let Some(trailers) = self.inner.trailers.take() {
            return Ok(Some(MetadataMap::from_headers(trailers)));
        }
        while self.message().await?.is_some() {}
        if let Some(trailers) = self.inner.trailers.take() {
            return Ok(Some(MetadataMap::from_headers(trailers)));
        }
        Ok(None)
    }

    fn decode_chunk(&mut self) -> Result<Option<T>, Status> {
        match self.inner.decode_chunk()? {
            Some(mut decode_buf) => match self.decoder.decode(&mut decode_buf)? {
                Some(msg) => {
                    self.inner.state = State::ReadHeader;
                    Ok(Some(msg))
                }
                None => Ok(None),
            },
            None => Ok(None),
        }
    }
}

impl StreamingInner {
    fn decode_chunk(&mut self) -> Result<Option<DecodeBuf<'_>>, Status> {
        if let State::ReadHeader = self.state {
            if self.buf.remaining() < HEADER_SIZE {
                return Ok(None);
            }

            let compression_flag = self.buf.get_u8();
            let compression = match compression_flag {
                0 => None,
                1 => {
                    if let Some(encoding) = self.encoding {
                        Some(encoding)
                    } else {
                        return Err(Status::internal(
                            "compressed message received but no grpc-encoding was specified",
                        ));
                    }
                }
                f => {
                    return Err(Status::internal(format!(
                        "invalid compression flag: {f} (valid: 0 or 1)"
                    )));
                }
            };

            let len = self.buf.get_u32() as usize;
            let limit = self
                .max_message_size
                .unwrap_or(DEFAULT_MAX_RECV_MESSAGE_SIZE);
            if len > limit {
                return Err(Status::out_of_range(format!(
                    "Error, decoded message length too large: found {len} bytes, the limit is: {limit} bytes"
                )));
            }

            self.buf.reserve(len);
            self.state = State::ReadBody { compression, len };
        }

        if let State::ReadBody { len, compression } = self.state {
            if self.buf.remaining() < len {
                return Ok(None);
            }

            let decode_buf = if let Some(encoding) = compression {
                self.decompress_buf.clear();
                let compressed_data = &self.buf[..len];
                super::compression::decompress(encoding, compressed_data, &mut self.decompress_buf)
                    .map_err(|err| Status::internal(format!("decompression error: {err}")))?;
                self.buf.advance(len);
                let decompressed_len = self.decompress_buf.len();
                DecodeBuf::new(&mut self.decompress_buf, decompressed_len)
            } else {
                DecodeBuf::new(&mut self.buf, len)
            };

            return Ok(Some(decode_buf));
        }

        Ok(None)
    }

    fn poll_frame(&mut self, cx: &mut Context<'_>) -> Poll<Result<Option<()>, Status>> {
        let frame = match ready!(Pin::new(&mut self.body).poll_frame(cx)) {
            Some(Ok(frame)) => frame,
            Some(Err(status)) => {
                if self.direction == Direction::Request && status.code() == Code::Cancelled {
                    return Poll::Ready(Ok(None));
                }
                self.state = State::Error(Some(status.clone()));
                // TODO(refactor): clone needed because status is both stored and returned.
                // Consider using Arc<Status> or splitting the error path.
                return Poll::Ready(Err(status));
            }
            None => {
                return Poll::Ready(if self.buf.has_remaining() {
                    trace!("unexpected EOF decoding stream");
                    Err(Status::internal("Unexpected EOF decoding stream."))
                } else {
                    Ok(None)
                });
            }
        };

        Poll::Ready(if frame.is_data() {
            self.buf.put(frame.into_data().unwrap());
            Ok(Some(()))
        } else if frame.is_trailers() {
            if let Some(trailers) = &mut self.trailers {
                trailers.extend(frame.into_trailers().unwrap());
            } else {
                self.trailers = Some(frame.into_trailers().unwrap());
            }
            Ok(None)
        } else {
            Ok(None)
        })
    }

    fn response(&mut self) -> Result<(), Status> {
        if let Direction::Response(status) = self.direction {
            if let Err(Some(e)) = crate::status::infer_grpc_status(self.trailers.as_ref(), status) {
                self.trailers.take();
                return Err(e);
            }
        }
        Ok(())
    }
}

impl<T> Stream for Streaming<T> {
    type Item = Result<T, Status>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            if let State::Error(status) = &mut self.inner.state {
                return Poll::Ready(status.take().map(Err));
            }

            if let Some(item) = self.decode_chunk()? {
                return Poll::Ready(Some(Ok(item)));
            }

            if ready!(self.inner.poll_frame(cx))?.is_none() {
                match self.inner.response() {
                    Ok(()) => return Poll::Ready(None),
                    Err(err) => self.inner.state = State::Error(Some(err)),
                }
            }
        }
    }
}

impl<T> fmt::Debug for Streaming<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Streaming").finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use http_body::Frame;
    use http_body_util::StreamBody;

    /// Trivial decoder that returns raw bytes as Vec<u8>.
    struct RawDecoder;

    impl Decoder for RawDecoder {
        type Item = Vec<u8>;
        type Error = Status;

        fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Vec<u8>>, Status> {
            let len = src.remaining();
            if len == 0 {
                return Ok(None);
            }
            let mut data = vec![0u8; len];
            src.copy_to_slice(&mut data);
            Ok(Some(data))
        }
    }

    /// Build a gRPC frame: [compression_flag: u8][length: u32 BE][payload].
    fn grpc_frame(compression_flag: u8, payload: &[u8]) -> Bytes {
        let mut buf = BytesMut::with_capacity(HEADER_SIZE + payload.len());
        buf.put_u8(compression_flag);
        buf.put_u32(payload.len() as u32);
        buf.put_slice(payload);
        buf.freeze()
    }

    /// Create a Streaming from a list of data frames.
    fn streaming_from_frames(frames: Vec<Bytes>) -> Streaming<Vec<u8>> {
        let body_stream =
            tokio_stream::iter(frames.into_iter().map(|b| Ok::<_, Status>(Frame::data(b))));
        let body = StreamBody::new(body_stream);
        Streaming::new_response(RawDecoder, body, StatusCode::OK, None, None)
    }

    /// Create a Streaming from data frames + trailer frame.
    fn streaming_with_trailers(frames: Vec<Bytes>, trailers: HeaderMap) -> Streaming<Vec<u8>> {
        let mut items: Vec<Result<Frame<Bytes>, Status>> =
            frames.into_iter().map(|b| Ok(Frame::data(b))).collect();
        items.push(Ok(Frame::trailers(trailers)));
        let body_stream = tokio_stream::iter(items);
        let body = StreamBody::new(body_stream);
        Streaming::new_response(RawDecoder, body, StatusCode::OK, None, None)
    }

    // ---------------------------------------------------------------
    // decode_chunk: ReadHeader state
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn decode_single_uncompressed_message() {
        let payload = b"hello";
        let frame = grpc_frame(0, payload);
        let mut streaming = streaming_from_frames(vec![frame]);

        let msg = streaming.message().await.unwrap().unwrap();
        assert_eq!(msg, b"hello");

        // No more messages
        assert!(streaming.message().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn decode_multiple_messages_in_one_frame() {
        let mut buf = BytesMut::new();
        // Two messages packed into a single body frame
        buf.extend_from_slice(&grpc_frame(0, b"first"));
        buf.extend_from_slice(&grpc_frame(0, b"second"));
        let frame = buf.freeze();

        let mut streaming = streaming_from_frames(vec![frame]);

        let msg1 = streaming.message().await.unwrap().unwrap();
        assert_eq!(msg1, b"first");
        let msg2 = streaming.message().await.unwrap().unwrap();
        assert_eq!(msg2, b"second");
        assert!(streaming.message().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn decode_message_split_across_frames() {
        // Header in first frame, payload in second
        let mut header = BytesMut::new();
        header.put_u8(0); // no compression
        header.put_u32(5); // length = 5

        let part1 = header.freeze();
        let part2 = Bytes::from_static(b"hello");

        let mut streaming = streaming_from_frames(vec![part1, part2]);

        let msg = streaming.message().await.unwrap().unwrap();
        assert_eq!(msg, b"hello");
    }

    // ---------------------------------------------------------------
    // decode_chunk: compression flags
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn decode_compressed_flag_without_encoding_errors() {
        // Compression flag=1 but no encoding specified
        let frame = grpc_frame(1, b"data");
        let mut streaming = streaming_from_frames(vec![frame]);

        let err = streaming.message().await.unwrap_err();
        assert_eq!(err.code(), Code::Internal);
        assert!(err.message().contains("no grpc-encoding"));
    }

    #[tokio::test]
    async fn decode_invalid_compression_flag() {
        // Compression flag=2 (invalid)
        let frame = grpc_frame(2, b"data");
        let mut streaming = streaming_from_frames(vec![frame]);

        let err = streaming.message().await.unwrap_err();
        assert_eq!(err.code(), Code::Internal);
        assert!(err.message().contains("invalid compression flag: 2"));
    }

    #[tokio::test]
    async fn decode_invalid_compression_flag_255() {
        let frame = grpc_frame(255, b"data");
        let mut streaming = streaming_from_frames(vec![frame]);

        let err = streaming.message().await.unwrap_err();
        assert_eq!(err.code(), Code::Internal);
        assert!(err.message().contains("invalid compression flag: 255"));
    }

    // ---------------------------------------------------------------
    // decode_chunk: message size limits
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn decode_message_exceeds_size_limit() {
        let payload = vec![0u8; 100];
        let frame = grpc_frame(0, &payload);

        // Create streaming with 50-byte limit
        let body_stream = tokio_stream::iter(vec![Ok::<_, Status>(Frame::data(frame))]);
        let body = StreamBody::new(body_stream);
        let mut streaming =
            Streaming::new_response(RawDecoder, body, StatusCode::OK, None, Some(50));

        let err = streaming.message().await.unwrap_err();
        assert_eq!(err.code(), Code::OutOfRange);
        assert!(err.message().contains("100 bytes"));
        assert!(err.message().contains("50 bytes"));
    }

    #[tokio::test]
    async fn decode_message_at_exact_size_limit() {
        let payload = vec![0u8; 50];
        let frame = grpc_frame(0, &payload);

        let body_stream = tokio_stream::iter(vec![Ok::<_, Status>(Frame::data(frame))]);
        let body = StreamBody::new(body_stream);
        let mut streaming =
            Streaming::new_response(RawDecoder, body, StatusCode::OK, None, Some(50));

        // Exactly at limit should succeed
        let msg = streaming.message().await.unwrap().unwrap();
        assert_eq!(msg.len(), 50);
    }

    // ---------------------------------------------------------------
    // decode_chunk: with compression
    // ---------------------------------------------------------------

    #[cfg(feature = "gzip")]
    #[tokio::test]
    async fn decode_gzip_compressed_message() {
        use super::super::compression::{compress, CompressionEncoding};

        let original = b"hello compressed world";
        let mut compressed = BytesMut::new();
        compress(CompressionEncoding::Gzip, original, &mut compressed).unwrap();

        let frame = grpc_frame(1, &compressed);
        let body_stream = tokio_stream::iter(vec![Ok::<_, Status>(Frame::data(frame))]);
        let body = StreamBody::new(body_stream);
        let mut streaming = Streaming::new_response(
            RawDecoder,
            body,
            StatusCode::OK,
            Some(CompressionEncoding::Gzip),
            None,
        );

        let msg = streaming.message().await.unwrap().unwrap();
        assert_eq!(msg, original);
    }

    // ---------------------------------------------------------------
    // poll_frame: body errors
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn decode_body_error_on_request() {
        // Request direction: Cancelled errors become None (EOF)
        let body_stream = tokio_stream::iter(vec![Err::<Frame<Bytes>, Status>(Status::cancelled(
            "client gone",
        ))]);
        let body = StreamBody::new(body_stream);
        let mut streaming = Streaming::new_request(RawDecoder, body, None, None);

        // Cancelled on request → graceful EOF
        assert!(streaming.message().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn decode_body_error_on_response() {
        // Response direction: errors propagate
        let body_stream = tokio_stream::iter(vec![Err::<Frame<Bytes>, Status>(
            Status::unavailable("server down"),
        )]);
        let body = StreamBody::new(body_stream);
        let mut streaming = Streaming::new_response(RawDecoder, body, StatusCode::OK, None, None);

        let err = streaming.message().await.unwrap_err();
        assert_eq!(err.code(), Code::Unavailable);
    }

    // ---------------------------------------------------------------
    // poll_frame: EOF with partial data
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn decode_unexpected_eof_with_partial_header() {
        // Only 3 bytes — not enough for a 5-byte header
        let partial = Bytes::from_static(&[0, 0, 0]);
        let mut streaming = streaming_from_frames(vec![partial]);

        let err = streaming.message().await.unwrap_err();
        assert_eq!(err.code(), Code::Internal);
        assert!(err.message().contains("Unexpected EOF"));
    }

    #[tokio::test]
    async fn decode_clean_eof_empty_body() {
        let mut streaming = streaming_from_frames(vec![]);
        assert!(streaming.message().await.unwrap().is_none());
    }

    // ---------------------------------------------------------------
    // trailers
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn decode_with_trailers() {
        let frame = grpc_frame(0, b"msg");
        let mut trailers = HeaderMap::new();
        trailers.insert("grpc-status", "0".parse().unwrap());
        trailers.insert("custom-trailer", "value".parse().unwrap());

        let mut streaming = streaming_with_trailers(vec![frame], trailers);

        let msg = streaming.message().await.unwrap().unwrap();
        assert_eq!(msg, b"msg");

        // Drain remaining
        assert!(streaming.message().await.unwrap().is_none());

        // Check trailers
        let t = streaming.trailers().await.unwrap().unwrap();
        assert_eq!(t.get("custom-trailer").unwrap(), "value");
    }

    #[tokio::test]
    async fn trailers_method_drains_remaining_messages() {
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&grpc_frame(0, b"m1"));
        buf.extend_from_slice(&grpc_frame(0, b"m2"));
        let data = buf.freeze();

        let mut trailers = HeaderMap::new();
        trailers.insert("grpc-status", "0".parse().unwrap());

        let mut streaming = streaming_with_trailers(vec![data], trailers);

        // Call trailers() directly without consuming messages first
        let t = streaming.trailers().await.unwrap().unwrap();
        assert!(t.get("grpc-status").is_some());
    }

    // ---------------------------------------------------------------
    // Error state machine
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn error_state_persists() {
        let frame = grpc_frame(2, b"bad"); // invalid flag
        let mut streaming = streaming_from_frames(vec![frame]);

        // First call: error from invalid compression flag
        let err = streaming.message().await.unwrap_err();
        assert_eq!(err.code(), Code::Internal);
        assert!(err.message().contains("invalid compression flag"));

        // Second call: also error (unexpected EOF since body was consumed)
        let err2 = streaming.message().await.unwrap_err();
        assert_eq!(err2.code(), Code::Internal);
    }

    // ---------------------------------------------------------------
    // Constructors
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn new_empty_returns_none_immediately() {
        let body_stream = tokio_stream::iter(Vec::<Result<Frame<Bytes>, Status>>::new());
        let body = StreamBody::new(body_stream);
        let mut streaming = Streaming::new_empty(RawDecoder, body);

        assert!(streaming.message().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn new_request_constructor() {
        let frame = grpc_frame(0, b"request_data");
        let body_stream = tokio_stream::iter(vec![Ok::<_, Status>(Frame::data(frame))]);
        let body = StreamBody::new(body_stream);
        let mut streaming = Streaming::new_request(RawDecoder, body, None, None);

        let msg = streaming.message().await.unwrap().unwrap();
        assert_eq!(msg, b"request_data");
    }

    // ---------------------------------------------------------------
    // infer_grpc_status in response()
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn response_with_error_status_in_trailers() {
        let frame = grpc_frame(0, b"data");
        let mut trailers = HeaderMap::new();
        trailers.insert("grpc-status", "13".parse().unwrap()); // Internal
        trailers.insert("grpc-message", "something broke".parse().unwrap());

        let mut streaming = streaming_with_trailers(vec![frame], trailers);

        // First message succeeds
        let msg = streaming.message().await.unwrap().unwrap();
        assert_eq!(msg, b"data");

        // Next call hits trailers → infer error status
        let err = streaming.message().await.unwrap_err();
        assert_eq!(err.code(), Code::Internal);
    }

    #[test]
    fn streaming_is_debug() {
        fn assert_debug<T: fmt::Debug>() {}
        assert_debug::<Streaming<Vec<u8>>>();
    }
}
