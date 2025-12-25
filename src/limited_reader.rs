/// 有限读取器模块
///
/// 提供请求大小限制，防止内存耗尽攻击
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, ReadBuf};

/// 最大请求大小（默认 1MB）
pub const DEFAULT_MAX_REQUEST_SIZE: usize = 1024 * 1024;

/// HTTP 请求头最大大小（默认 8KB）
pub const DEFAULT_MAX_HEADER_SIZE: usize = 8 * 1024;

/// 有限读取器 - 限制可以读取的最大字节数
pub struct LimitedReader<R> {
    inner: R,
    remaining: usize,
    limit: usize,
}

impl<R> LimitedReader<R> {
    /// 创建新的有限读取器
    pub fn new(inner: R, limit: usize) -> Self {
        Self {
            inner,
            remaining: limit,
            limit,
        }
    }

    /// 使用默认限制（1MB）
    pub fn with_default_limit(inner: R) -> Self {
        Self::new(inner, DEFAULT_MAX_REQUEST_SIZE)
    }

    /// 使用 HTTP 头大小限制（8KB）
    pub fn with_header_limit(inner: R) -> Self {
        Self::new(inner, DEFAULT_MAX_HEADER_SIZE)
    }

    /// 获取剩余可读字节数
    pub fn remaining(&self) -> usize {
        self.remaining
    }

    /// 获取总限制
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// 获取已读取字节数
    pub fn read_count(&self) -> usize {
        self.limit - self.remaining
    }

    /// 获取内部读取器的引用
    pub fn get_ref(&self) -> &R {
        &self.inner
    }

    /// 获取内部读取器的可变引用
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.inner
    }

    /// 消费 LimitedReader，返回内部读取器
    pub fn into_inner(self) -> R {
        self.inner
    }

    /// 重置限制（允许继续读取）
    pub fn reset_limit(&mut self, new_limit: usize) {
        self.limit = new_limit;
        self.remaining = new_limit;
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for LimitedReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.remaining == 0 {
            return Poll::Ready(Err(io::Error::other(format!(
                "Request size limit exceeded: {} bytes (limit: {} bytes)",
                self.limit, self.limit
            ))));
        }

        //记录当前已填充的字节数
        let before = buf.filled().len();

        // 从内部读取器读取数据，但限制读取量
        let result = {
            // 临时限制缓冲区容量
            let limit = self.remaining.min(buf.remaining());
            let mut limited = buf.take(limit);
            Pin::new(&mut self.inner).poll_read(cx, &mut limited)
        };

        // 更新剩余字节数
        match result {
            Poll::Ready(Ok(())) => {
                let after = buf.filled().len();
                let read = after - before;
                self.remaining = self.remaining.saturating_sub(read);
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_limited_reader_creation() {
        struct DummyReader;
        impl AsyncRead for DummyReader {
            fn poll_read(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
                _buf: &mut ReadBuf<'_>,
            ) -> Poll<io::Result<()>> {
                Poll::Ready(Ok(()))
            }
        }

        let reader = DummyReader;
        let limited = LimitedReader::new(reader, 1024);
        assert_eq!(limited.limit(), 1024);
        assert_eq!(limited.remaining(), 1024);
        assert_eq!(limited.read_count(), 0);
    }

    #[test]
    fn test_limited_reader_read_count() {
        struct DummyReader;
        impl AsyncRead for DummyReader {
            fn poll_read(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
                _buf: &mut ReadBuf<'_>,
            ) -> Poll<io::Result<()>> {
                Poll::Ready(Ok(()))
            }
        }

        let reader = DummyReader;
        let mut limited = LimitedReader::new(reader, 100);

        assert_eq!(limited.read_count(), 0);
        assert_eq!(limited.limit(), 100);
        assert_eq!(limited.remaining(), 100);

        // 模拟读取
        limited.remaining = 90;
        assert_eq!(limited.read_count(), 10);
    }

    #[test]
    fn test_limited_reader_reset() {
        struct DummyReader;
        impl AsyncRead for DummyReader {
            fn poll_read(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
                _buf: &mut ReadBuf<'_>,
            ) -> Poll<io::Result<()>> {
                Poll::Ready(Ok(()))
            }
        }

        let reader = DummyReader;
        let mut limited = LimitedReader::new(reader, 100);

        // 模拟读取
        limited.remaining = 50;
        assert_eq!(limited.remaining(), 50);

        // 重置限制
        limited.reset_limit(200);
        assert_eq!(limited.remaining(), 200);
        assert_eq!(limited.limit(), 200);
    }

    #[test]
    fn test_limited_reader_constants() {
        assert_eq!(DEFAULT_MAX_REQUEST_SIZE, 1024 * 1024);
        assert_eq!(DEFAULT_MAX_HEADER_SIZE, 8 * 1024);
    }

    #[test]
    fn test_limited_reader_with_limits() {
        struct DummyReader;
        impl AsyncRead for DummyReader {
            fn poll_read(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
                _buf: &mut ReadBuf<'_>,
            ) -> Poll<io::Result<()>> {
                Poll::Ready(Ok(()))
            }
        }

        let reader1 = DummyReader;
        let limited1 = LimitedReader::with_default_limit(reader1);
        assert_eq!(limited1.limit(), DEFAULT_MAX_REQUEST_SIZE);

        let reader2 = DummyReader;
        let limited2 = LimitedReader::with_header_limit(reader2);
        assert_eq!(limited2.limit(), DEFAULT_MAX_HEADER_SIZE);
    }
}
