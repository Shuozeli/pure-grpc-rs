//! gRPC-Web body wrapper that handles frame encoding/decoding and
//! trailers-in-body for the gRPC-Web protocol.

use crate::{Encoding, GRPC_HEADER_SIZE};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use bytes::{BufMut, Bytes, BytesMut};
use http::HeaderMap;
use http_body::Frame;
use pin_project_lite::pin_project;
use std::pin::Pin;
use std::task::{Context, Poll};

pin_project! {
    /// Body wrapper that translates between gRPC and gRPC-Web framing.
    ///
    /// For **request decoding**: strips base64 encoding (if text mode) so the inner
    /// gRPC service receives standard binary frames.
    ///
    /// For **response encoding**: encodes trailers into the body (gRPC-Web spec) and
    /// optionally base64-encodes the output (for text mode).
    pub struct GrpcWebCall<B> {
        #[pin]
        inner: B,
        encoding: Encoding,
        direction: Direction,
        buf: BytesMut,
        // For response: accumulate trailers and append to body
        trailers: Option<HeaderMap>,
        trailers_sent: bool,
    }
}

#[derive(Debug, Clone, Copy)]
enum Direction {
    Decode,
    Encode,
}

impl<B> GrpcWebCall<B> {
    /// Wrap a request body for decoding (client->server).
    pub(crate) fn request(inner: B, encoding: Encoding) -> Self {
        Self {
            inner,
            encoding,
            direction: Direction::Decode,
            buf: BytesMut::with_capacity(8192),
            trailers: None,
            trailers_sent: false,
        }
    }

    /// Wrap a response body for encoding (server->client).
    pub(crate) fn response(inner: B, encoding: Encoding) -> Self {
        Self {
            inner,
            encoding,
            direction: Direction::Encode,
            buf: BytesMut::with_capacity(8192),
            trailers: None,
            trailers_sent: false,
        }
    }
}

impl<B> http_body::Body for GrpcWebCall<B>
where
    B: http_body::Body<Data = Bytes> + Unpin,
    B::Error: std::fmt::Display,
{
    type Data = Bytes;
    type Error = B::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.project();

        match this.direction {
            Direction::Decode => poll_decode(this.inner, this.encoding, this.buf, cx),
            Direction::Encode => poll_encode(
                this.inner,
                this.encoding,
                this.buf,
                this.trailers,
                this.trailers_sent,
                cx,
            ),
        }
    }
}

/// Decode a gRPC-Web request body into standard gRPC frames.
fn poll_decode<B>(
    inner: Pin<&mut B>,
    encoding: &Encoding,
    buf: &mut BytesMut,
    cx: &mut Context<'_>,
) -> Poll<Option<Result<Frame<Bytes>, B::Error>>>
where
    B: http_body::Body<Data = Bytes>,
{
    match inner.poll_frame(cx) {
        Poll::Ready(Some(Ok(frame))) => {
            if let Some(data) = frame.data_ref() {
                match encoding {
                    Encoding::Base64 => {
                        match BASE64.decode(data) {
                            Ok(decoded) => {
                                Poll::Ready(Some(Ok(Frame::data(Bytes::from(decoded)))))
                            }
                            Err(_) => {
                                // Partial base64 — buffer and wait for more
                                buf.extend_from_slice(data);
                                // Try to decode what we have (4-byte aligned)
                                let aligned_len = buf.len() / 4 * 4;
                                if aligned_len > 0 {
                                    let chunk = buf.split_to(aligned_len);
                                    match BASE64.decode(&chunk) {
                                        Ok(decoded) => Poll::Ready(Some(Ok(Frame::data(
                                            Bytes::from(decoded),
                                        )))),
                                        Err(_) => Poll::Ready(Some(Ok(Frame::data(
                                            chunk.freeze(),
                                        )))),
                                    }
                                } else {
                                    cx.waker().wake_by_ref();
                                    Poll::Pending
                                }
                            }
                        }
                    }
                    Encoding::Binary => Poll::Ready(Some(Ok(frame))),
                }
            } else if frame.trailers_ref().is_some() {
                // gRPC-Web requests shouldn't have HTTP trailers, pass through
                Poll::Ready(Some(Ok(frame)))
            } else {
                Poll::Ready(Some(Ok(frame)))
            }
        }
        other => other,
    }
}

/// Encode a gRPC response body into gRPC-Web format.
///
/// Trailers are encoded as a frame in the response body with the trailer flag (0x80) set.
fn poll_encode<B>(
    inner: Pin<&mut B>,
    encoding: &Encoding,
    _buf: &mut BytesMut,
    trailers: &mut Option<HeaderMap>,
    trailers_sent: &mut bool,
    cx: &mut Context<'_>,
) -> Poll<Option<Result<Frame<Bytes>, B::Error>>>
where
    B: http_body::Body<Data = Bytes> + Unpin,
    B::Error: std::fmt::Display,
{
    // If we've already captured trailers, encode and send them
    if let Some(trailer_map) = trailers.take() {
        *trailers_sent = true;
        let encoded = encode_trailers(&trailer_map, encoding);
        return Poll::Ready(Some(Ok(Frame::data(encoded))));
    }

    if *trailers_sent {
        return Poll::Ready(None);
    }

    match inner.poll_frame(cx) {
        Poll::Ready(Some(Ok(frame))) => {
            if let Some(data) = frame.data_ref() {
                // Encode data frame
                let output = match encoding {
                    Encoding::Base64 => Bytes::from(BASE64.encode(data)),
                    Encoding::Binary => data.clone(),
                };
                Poll::Ready(Some(Ok(Frame::data(output))))
            } else if let Some(trailer_map) = frame.trailers_ref() {
                // Capture trailers — they'll be sent as a body frame on next poll
                *trailers = Some(trailer_map.clone());
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready(Some(Ok(frame)))
            }
        }
        Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e))),
        Poll::Ready(None) => {
            // Inner body done — send empty trailers if not yet sent
            if !*trailers_sent {
                *trailers_sent = true;
                let empty_trailers = HeaderMap::new();
                let encoded = encode_trailers(&empty_trailers, encoding);
                if !encoded.is_empty() {
                    return Poll::Ready(Some(Ok(Frame::data(encoded))));
                }
            }
            Poll::Ready(None)
        }
        Poll::Pending => Poll::Pending,
    }
}

/// Encode HTTP trailers as a gRPC-Web trailer frame.
///
/// Format: `[0x80][u32 length][HTTP/1-style headers]`
fn encode_trailers(trailers: &HeaderMap, encoding: &Encoding) -> Bytes {
    let mut trailer_buf = BytesMut::new();
    for (key, value) in trailers.iter() {
        trailer_buf.extend_from_slice(key.as_str().as_bytes());
        trailer_buf.extend_from_slice(b":");
        trailer_buf.extend_from_slice(value.as_bytes());
        trailer_buf.extend_from_slice(b"\r\n");
    }

    let trailer_len = trailer_buf.len();

    // Build the frame: flag byte (0x80 = trailer) + length + data
    let mut frame = BytesMut::with_capacity(GRPC_HEADER_SIZE + trailer_len);
    frame.put_u8(0x80); // Trailer flag
    frame.put_u32(trailer_len as u32);
    frame.extend_from_slice(&trailer_buf);

    match encoding {
        Encoding::Base64 => Bytes::from(BASE64.encode(&frame)),
        Encoding::Binary => frame.freeze(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_trailers_binary() {
        let mut trailers = HeaderMap::new();
        trailers.insert("grpc-status", "0".parse().unwrap());
        let frame = encode_trailers(&trailers, &Encoding::Binary);

        // Flag byte should be 0x80
        assert_eq!(frame[0], 0x80);

        // Length (4 bytes big-endian)
        let len = u32::from_be_bytes([frame[1], frame[2], frame[3], frame[4]]) as usize;

        // Trailer content
        let content = &frame[GRPC_HEADER_SIZE..GRPC_HEADER_SIZE + len];
        let content_str = std::str::from_utf8(content).unwrap();
        assert!(content_str.contains("grpc-status:0"));
    }

    #[test]
    fn encode_trailers_base64() {
        let mut trailers = HeaderMap::new();
        trailers.insert("grpc-status", "0".parse().unwrap());
        let frame = encode_trailers(&trailers, &Encoding::Base64);

        // Should be valid base64
        let decoded = BASE64.decode(&frame).unwrap();
        assert_eq!(decoded[0], 0x80);
    }

    #[test]
    fn encode_empty_trailers() {
        let trailers = HeaderMap::new();
        let frame = encode_trailers(&trailers, &Encoding::Binary);
        assert_eq!(frame[0], 0x80);
        assert_eq!(
            u32::from_be_bytes([frame[1], frame[2], frame[3], frame[4]]),
            0
        );
    }
}
