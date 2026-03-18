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
        match self.inner.decode_chunk(self.decoder.buffer_settings())? {
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
    fn decode_chunk(
        &mut self,
        _buffer_settings: super::BufferSettings,
    ) -> Result<Option<DecodeBuf<'_>>, Status> {
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
                let _ = std::mem::replace(&mut self.state, State::Error(Some(status.clone())));
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
