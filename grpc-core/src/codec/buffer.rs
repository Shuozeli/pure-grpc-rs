use bytes::buf::UninitSlice;
use bytes::{Buf, BufMut, Bytes, BytesMut};

/// A specialized buffer to decode gRPC messages from.
#[derive(Debug)]
pub struct DecodeBuf<'a> {
    buf: &'a mut BytesMut,
    len: usize,
}

/// A specialized buffer to encode gRPC messages into.
#[derive(Debug)]
pub struct EncodeBuf<'a> {
    buf: &'a mut BytesMut,
}

impl<'a> DecodeBuf<'a> {
    pub fn new(buf: &'a mut BytesMut, len: usize) -> Self {
        DecodeBuf { buf, len }
    }
}

impl Buf for DecodeBuf<'_> {
    fn remaining(&self) -> usize {
        self.len
    }

    fn chunk(&self) -> &[u8] {
        let ret = self.buf.chunk();
        if ret.len() > self.len {
            &ret[..self.len]
        } else {
            ret
        }
    }

    fn advance(&mut self, cnt: usize) {
        assert!(cnt <= self.len);
        self.buf.advance(cnt);
        self.len -= cnt;
    }

    fn copy_to_bytes(&mut self, len: usize) -> Bytes {
        assert!(len <= self.len);
        self.len -= len;
        self.buf.copy_to_bytes(len)
    }
}

impl<'a> EncodeBuf<'a> {
    pub fn new(buf: &'a mut BytesMut) -> Self {
        EncodeBuf { buf }
    }

    pub fn reserve(&mut self, additional: usize) {
        self.buf.reserve(additional);
    }
}

unsafe impl BufMut for EncodeBuf<'_> {
    fn remaining_mut(&self) -> usize {
        self.buf.remaining_mut()
    }

    unsafe fn advance_mut(&mut self, cnt: usize) {
        unsafe { self.buf.advance_mut(cnt) }
    }

    fn chunk_mut(&mut self) -> &mut UninitSlice {
        self.buf.chunk_mut()
    }

    fn put<T: Buf>(&mut self, src: T)
    where
        Self: Sized,
    {
        self.buf.put(src)
    }

    fn put_slice(&mut self, src: &[u8]) {
        self.buf.put_slice(src)
    }

    fn put_bytes(&mut self, val: u8, cnt: usize) {
        self.buf.put_bytes(val, cnt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_buf_limits_to_len() {
        let mut payload = BytesMut::with_capacity(100);
        payload.put(&vec![0u8; 50][..]);
        let mut buf = DecodeBuf::new(&mut payload, 20);

        assert_eq!(buf.remaining(), 20);
        assert_eq!(buf.chunk().len(), 20);

        buf.advance(10);
        assert_eq!(buf.remaining(), 10);

        let bytes = buf.copy_to_bytes(5);
        assert_eq!(bytes.len(), 5);
        assert_eq!(buf.remaining(), 5);
    }

    #[test]
    fn encode_buf_writes() {
        let mut bytes = BytesMut::with_capacity(100);
        let mut buf = EncodeBuf::new(&mut bytes);
        buf.put_slice(b"hello");
        assert_eq!(&bytes[..], b"hello");
    }
}
