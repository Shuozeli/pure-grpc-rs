use super::compression::CompressionEncoding;
use super::{EncodeBuf, Encoder, DEFAULT_MAX_SEND_MESSAGE_SIZE, HEADER_SIZE};
use crate::Status;
use bytes::{BufMut, Bytes, BytesMut};
use http::HeaderMap;
use http_body::{Body, Frame};
use pin_project_lite::pin_project;
use std::{
    pin::Pin,
    task::{ready, Context, Poll},
};
use tokio_stream::{adapters::Fuse, Stream, StreamExt};

pin_project! {
    /// Stream combinator that encodes messages into gRPC-framed bytes.
    ///
    /// Batches multiple small messages until the buffer exceeds `yield_threshold`.
    #[derive(Debug)]
    struct EncodedBytes<T, U> {
        #[pin]
        source: Fuse<U>,
        encoder: T,
        compression_encoding: Option<CompressionEncoding>,
        max_message_size: Option<usize>,
        buf: BytesMut,
        uncompression_buf: BytesMut,
        error: Option<Status>,
    }
}

impl<T: Encoder, U: Stream> EncodedBytes<T, U> {
    fn new(
        encoder: T,
        source: U,
        compression_encoding: Option<CompressionEncoding>,
        max_message_size: Option<usize>,
    ) -> Self {
        let buffer_settings = encoder.buffer_settings();
        let buf = BytesMut::with_capacity(buffer_settings.buffer_size);
        let uncompression_buf = if compression_encoding.is_some() {
            BytesMut::with_capacity(buffer_settings.buffer_size)
        } else {
            BytesMut::new()
        };
        Self {
            source: source.fuse(),
            encoder,
            compression_encoding,
            max_message_size,
            buf,
            uncompression_buf,
            error: None,
        }
    }
}

impl<T, U> Stream for EncodedBytes<T, U>
where
    T: Encoder<Error = Status>,
    U: Stream<Item = Result<T::Item, Status>>,
{
    type Item = Result<Bytes, Status>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        let buffer_settings = this.encoder.buffer_settings();

        if let Some(status) = this.error.take() {
            return Poll::Ready(Some(Err(status)));
        }

        loop {
            match this.source.as_mut().poll_next(cx) {
                Poll::Pending if this.buf.is_empty() => return Poll::Pending,
                Poll::Ready(None) if this.buf.is_empty() => return Poll::Ready(None),
                Poll::Pending | Poll::Ready(None) => {
                    return Poll::Ready(Some(Ok(this.buf.split_to(this.buf.len()).freeze())));
                }
                Poll::Ready(Some(Ok(item))) => {
                    if let Err(status) = encode_item(
                        this.encoder,
                        this.buf,
                        this.uncompression_buf,
                        *this.compression_encoding,
                        *this.max_message_size,
                        item,
                    ) {
                        return Poll::Ready(Some(Err(status)));
                    }
                    if this.buf.len() >= buffer_settings.yield_threshold {
                        return Poll::Ready(Some(Ok(this.buf.split_to(this.buf.len()).freeze())));
                    }
                }
                Poll::Ready(Some(Err(status))) => {
                    if this.buf.is_empty() {
                        return Poll::Ready(Some(Err(status)));
                    }
                    *this.error = Some(status);
                    return Poll::Ready(Some(Ok(this.buf.split_to(this.buf.len()).freeze())));
                }
            }
        }
    }
}

fn encode_item<T>(
    encoder: &mut T,
    buf: &mut BytesMut,
    uncompression_buf: &mut BytesMut,
    compression_encoding: Option<CompressionEncoding>,
    max_message_size: Option<usize>,
    item: T::Item,
) -> Result<(), Status>
where
    T: Encoder<Error = Status>,
{
    let offset = buf.len();

    // Reserve space for the 5-byte gRPC frame header.
    buf.reserve(HEADER_SIZE);
    // SAFETY: We just reserved at least HEADER_SIZE bytes above, so advancing
    // the write cursor by HEADER_SIZE stays within the allocated capacity.
    // The header bytes are written by `finish_encoding` before the buffer is read.
    unsafe {
        buf.advance_mut(HEADER_SIZE);
    }

    let encode_result = if let Some(encoding) = compression_encoding {
        // Encode into temp buffer, then compress into main buffer
        uncompression_buf.clear();
        encoder
            .encode(item, &mut EncodeBuf::new(uncompression_buf))
            .map_err(|err| Status::internal(format!("Error encoding: {err}")))
            .and_then(|()| {
                super::compression::compress(encoding, uncompression_buf, buf)
                    .map_err(|err| Status::internal(format!("Error compressing: {err}")))
            })
    } else {
        // Encode directly (no compression)
        encoder
            .encode(item, &mut EncodeBuf::new(buf))
            .map_err(|err| Status::internal(format!("Error encoding: {err}")))
    };

    if let Err(status) = encode_result {
        // Truncate back to discard the uninitialized header and any partial payload
        buf.truncate(offset);
        return Err(status);
    }

    // Backfill the header now that we know the length
    let result = finish_encoding(compression_encoding, max_message_size, &mut buf[offset..]);
    if result.is_err() {
        buf.truncate(offset);
    }
    result
}

fn finish_encoding(
    compression_encoding: Option<CompressionEncoding>,
    max_message_size: Option<usize>,
    buf: &mut [u8],
) -> Result<(), Status> {
    let len = buf.len() - HEADER_SIZE;
    let limit = max_message_size.unwrap_or(DEFAULT_MAX_SEND_MESSAGE_SIZE);

    if len > limit {
        return Err(Status::out_of_range(format!(
            "Error, encoded message length too large: found {len} bytes, the limit is: {limit} bytes"
        )));
    }

    if len > u32::MAX as usize {
        return Err(Status::resource_exhausted(format!(
            "Cannot return body with more than 4GB of data but got {len} bytes"
        )));
    }

    {
        let mut header = &mut buf[..HEADER_SIZE];
        header.put_u8(compression_encoding.is_some() as u8);
        header.put_u32(len as u32);
    }

    Ok(())
}

#[derive(Debug)]
enum Role {
    Client,
    Server,
}

pin_project! {
    /// HTTP body that wraps encoded gRPC frames.
    ///
    /// Server variant appends gRPC status trailers on stream end.
    /// Client variant does not send trailers.
    #[derive(Debug)]
    pub struct EncodeBody<T, U> {
        #[pin]
        inner: EncodedBytes<T, U>,
        state: EncodeState,
    }
}

#[derive(Debug)]
struct EncodeState {
    error: Option<Status>,
    role: Role,
    is_end_stream: bool,
}

impl<T: Encoder, U: Stream> EncodeBody<T, U> {
    pub fn new_client(
        encoder: T,
        source: U,
        compression_encoding: Option<CompressionEncoding>,
        max_message_size: Option<usize>,
    ) -> Self {
        Self {
            inner: EncodedBytes::new(encoder, source, compression_encoding, max_message_size),
            state: EncodeState {
                error: None,
                role: Role::Client,
                is_end_stream: false,
            },
        }
    }

    pub fn new_server(
        encoder: T,
        source: U,
        compression_encoding: Option<CompressionEncoding>,
        max_message_size: Option<usize>,
    ) -> Self {
        Self {
            inner: EncodedBytes::new(encoder, source, compression_encoding, max_message_size),
            state: EncodeState {
                error: None,
                role: Role::Server,
                is_end_stream: false,
            },
        }
    }
}

impl EncodeState {
    fn trailers(&mut self) -> Option<Result<HeaderMap, Status>> {
        match self.role {
            Role::Client => None,
            Role::Server => {
                if self.is_end_stream {
                    return None;
                }
                self.is_end_stream = true;
                let status = self.error.take().unwrap_or_else(|| Status::ok(""));
                Some(status.to_header_map())
            }
        }
    }
}

impl<T, U> Body for EncodeBody<T, U>
where
    T: Encoder<Error = Status>,
    U: Stream<Item = Result<T::Item, Status>>,
{
    type Data = Bytes;
    type Error = Status;

    fn is_end_stream(&self) -> bool {
        self.state.is_end_stream
    }

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.project();
        match ready!(this.inner.poll_next(cx)) {
            Some(Ok(data)) => Some(Ok(Frame::data(data))).into(),
            Some(Err(status)) => match this.state.role {
                Role::Client => Some(Err(status)).into(),
                Role::Server => {
                    this.state.is_end_stream = true;
                    Some(Ok(Frame::trailers(
                        status.to_header_map().unwrap_or_default(),
                    )))
                    .into()
                }
            },
            None => this.state.trailers().map(|t| t.map(Frame::trailers)).into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::TestEncoder;
    use super::*;

    #[test]
    fn encode_item_produces_valid_frame() {
        let mut encoder = TestEncoder;
        let mut buf = BytesMut::new();
        let mut uncomp_buf = BytesMut::new();
        let data = vec![1u8, 2, 3, 4, 5];

        encode_item(&mut encoder, &mut buf, &mut uncomp_buf, None, None, data).unwrap();

        assert_eq!(buf[0], 0); // no compression
        assert_eq!(&buf[1..5], &[0, 0, 0, 5]); // length = 5
        assert_eq!(&buf[5..], &[1, 2, 3, 4, 5]); // payload
    }

    #[test]
    fn encode_item_error_truncates_buffer() {
        // An encoder that always fails
        #[derive(Debug)]
        struct FailEncoder;

        impl Encoder for FailEncoder {
            type Item = Vec<u8>;
            type Error = Status;

            fn encode(
                &mut self,
                _item: Self::Item,
                _buf: &mut EncodeBuf<'_>,
            ) -> Result<(), Self::Error> {
                Err(Status::internal("boom"))
            }
        }

        let mut encoder = FailEncoder;
        let mut buf = BytesMut::new();
        buf.extend_from_slice(b"existing");
        let original_len = buf.len();

        let mut uncomp_buf = BytesMut::new();
        let result = encode_item(
            &mut encoder,
            &mut buf,
            &mut uncomp_buf,
            None,
            None,
            vec![1, 2, 3],
        );

        assert!(result.is_err());
        // Buffer must be truncated back — no partial header bytes left behind
        assert_eq!(buf.len(), original_len);
        assert_eq!(&buf[..], b"existing");
    }

    #[test]
    fn encode_item_respects_max_message_size() {
        let mut encoder = TestEncoder;
        let mut buf = BytesMut::new();
        let mut uncomp_buf = BytesMut::new();
        let data = vec![0u8; 100];

        let result = encode_item(
            &mut encoder,
            &mut buf,
            &mut uncomp_buf,
            None,
            Some(50),
            data,
        );
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), crate::Code::OutOfRange);
    }

    #[cfg(feature = "gzip")]
    #[test]
    fn encode_item_with_compression() {
        let mut encoder = TestEncoder;
        let mut buf = BytesMut::new();
        let mut uncomp_buf = BytesMut::new();
        let data = vec![42u8; 100];

        encode_item(
            &mut encoder,
            &mut buf,
            &mut uncomp_buf,
            Some(CompressionEncoding::Gzip),
            None,
            data,
        )
        .unwrap();

        assert_eq!(buf[0], 1); // compressed flag
                               // Compressed data should be smaller than 100 + HEADER_SIZE
        assert!(buf.len() < 100 + HEADER_SIZE);
    }

    #[tokio::test]
    async fn encode_body_server_produces_data_and_trailers() {
        use http_body::Body as _;
        use http_body_util::BodyExt;
        use std::pin::pin;

        let messages = vec![Ok(vec![1u8, 2, 3]), Ok(vec![4, 5, 6])];
        let source = tokio_stream::iter(messages);

        let mut body = pin!(EncodeBody::new_server(TestEncoder, source, None, None));

        // Should get data frames
        let frame1 = body.frame().await.unwrap().unwrap();
        assert!(frame1.is_data());

        // After data is exhausted, should get trailers with grpc-status: 0
        // (may need to drain more data frames first depending on batching)
        let mut got_trailers = false;
        while let Some(frame) = body.frame().await {
            let frame = frame.unwrap();
            if frame.is_trailers() {
                let trailers = frame.into_trailers().unwrap();
                assert_eq!(trailers.get("grpc-status").unwrap(), "0");
                got_trailers = true;
                break;
            }
        }
        assert!(got_trailers, "server body should end with trailers");
        assert!(body.is_end_stream());
    }

    #[tokio::test]
    async fn encode_body_client_no_trailers() {
        use http_body_util::BodyExt;
        use std::pin::pin;

        let messages = vec![Ok(vec![1u8, 2, 3])];
        let source = tokio_stream::iter(messages);

        let mut body = pin!(EncodeBody::new_client(TestEncoder, source, None, None));

        // Drain all frames
        let mut frame_count = 0;
        while let Some(frame) = body.frame().await {
            let frame = frame.unwrap();
            assert!(frame.is_data(), "client body should only have data frames");
            frame_count += 1;
        }
        assert!(frame_count > 0);
    }
}
