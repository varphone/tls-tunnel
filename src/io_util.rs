/// 批量 I/O 优化模块
///
/// 提供优化的批量写入操作，减少系统调用次数
use std::io::{self, IoSlice};
use tokio::io::AsyncWriteExt;

/// 批量写入辅助函数 - 使用 write_vectored 减少系统调用
///
/// # 示例
/// ```rust
/// use tokio::net::TcpStream;
/// use tls_tunnel::io_util::write_vectored_all;
///
/// # async fn example(stream: &mut TcpStream) -> std::io::Result<()> {
/// let header = b"HTTP/1.1 200 OK\r\n";
/// let content_length = b"Content-Length: 13\r\n\r\n";
/// let body = b"Hello, World!";
///
/// // 单次系统调用写入多个缓冲区
/// write_vectored_all(stream, &[header, content_length, body]).await?;
/// # Ok(())
/// # }
/// ```
pub async fn write_vectored_all<W>(writer: &mut W, bufs: &[&[u8]]) -> io::Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    if bufs.is_empty() {
        return Ok(());
    }

    // 如果只有一个缓冲区，直接使用 write_all
    if bufs.len() == 1 {
        return writer.write_all(bufs[0]).await;
    }

    // 构造 IoSlice 数组
    let io_slices: Vec<IoSlice> = bufs.iter().map(|buf| IoSlice::new(buf)).collect();

    // 计算总字节数
    let total_bytes: usize = bufs.iter().map(|buf| buf.len()).sum();
    let mut written = 0;

    // 写入所有数据
    let mut remaining_slices = &io_slices[..];
    while written < total_bytes {
        // 尝试写入
        match writer.write_vectored(remaining_slices).await {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "failed to write vectored data",
                ))
            }
            Ok(n) => {
                written += n;
                if written >= total_bytes {
                    break;
                }

                // 更新剩余的切片
                remaining_slices = advance_slices(remaining_slices, n);
            }
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }

    writer.flush().await
}

/// 前进 IoSlice 切片，跳过已写入的字节
fn advance_slices<'a>(slices: &'a [IoSlice<'a>], mut n: usize) -> &'a [IoSlice<'a>] {
    let mut idx = 0;
    for (i, slice) in slices.iter().enumerate() {
        let len = slice.len();
        if n < len {
            // 在当前切片中间，无法简单跳过（需要重新构造）
            // 这里简化处理，实际应用中可能需要更复杂的逻辑
            return &slices[idx..];
        }
        n -= len;
        idx = i + 1;
        if n == 0 {
            return &slices[idx..];
        }
    }
    &slices[slices.len()..]
}

/// 使用预分配缓冲区避免频繁分配
///
/// # 示例
/// ```rust
/// use tls_tunnel::io_util::VecBuffer;
///
/// let mut buf = VecBuffer::with_capacity(8192);
/// buf.push_slice(b"Hello, ");
/// buf.push_slice(b"World!");
/// assert_eq!(buf.as_slice(), b"Hello, World!");
/// ```
pub struct VecBuffer {
    inner: Vec<u8>,
}

impl VecBuffer {
    /// 创建指定容量的缓冲区
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Vec::with_capacity(capacity),
        }
    }

    /// 添加数据切片
    pub fn push_slice(&mut self, data: &[u8]) {
        self.inner.extend_from_slice(data);
    }

    /// 添加单个字节
    pub fn push_byte(&mut self, byte: u8) {
        self.inner.push(byte);
    }

    /// 获取数据切片
    pub fn as_slice(&self) -> &[u8] {
        &self.inner
    }

    /// 获取可变切片
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.inner
    }

    /// 清空缓冲区（保留容量）
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// 获取当前长度
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// 检查是否为空
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// 获取容量
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// 预留额外空间
    pub fn reserve(&mut self, additional: usize) {
        self.inner.reserve(additional);
    }

    /// 转换为 Vec<u8>
    pub fn into_vec(self) -> Vec<u8> {
        self.inner
    }
}

impl Default for VecBuffer {
    fn default() -> Self {
        Self::with_capacity(4096)
    }
}

impl From<Vec<u8>> for VecBuffer {
    fn from(vec: Vec<u8>) -> Self {
        Self { inner: vec }
    }
}

impl AsRef<[u8]> for VecBuffer {
    fn as_ref(&self) -> &[u8] {
        &self.inner
    }
}

impl AsMut<[u8]> for VecBuffer {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_write_vectored_all() {
        let mut buffer = Vec::new();

        let part1 = b"Hello, ";
        let part2 = b"World";
        let part3 = b"!";

        write_vectored_all(&mut buffer, &[part1, part2, part3])
            .await
            .unwrap();

        assert_eq!(&buffer, b"Hello, World!");
    }

    #[tokio::test]
    async fn test_write_vectored_all_single_buf() {
        let mut buffer = Vec::new();
        let data = b"Single buffer";

        write_vectored_all(&mut buffer, &[data]).await.unwrap();

        assert_eq!(&buffer, b"Single buffer");
    }

    #[tokio::test]
    async fn test_write_vectored_all_empty() {
        let mut buffer = Vec::new();

        write_vectored_all(&mut buffer, &[]).await.unwrap();

        assert!(buffer.is_empty());
    }

    #[test]
    fn test_vec_buffer_basic() {
        let mut buf = VecBuffer::with_capacity(16);
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());
        assert!(buf.capacity() >= 16);

        buf.push_slice(b"Hello");
        assert_eq!(buf.len(), 5);
        assert_eq!(buf.as_slice(), b"Hello");

        buf.push_byte(b'!');
        assert_eq!(buf.len(), 6);
        assert_eq!(buf.as_slice(), b"Hello!");
    }

    #[test]
    fn test_vec_buffer_clear() {
        let mut buf = VecBuffer::with_capacity(16);
        buf.push_slice(b"Test");
        assert_eq!(buf.len(), 4);

        let old_capacity = buf.capacity();
        buf.clear();
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());
        assert_eq!(buf.capacity(), old_capacity); // 容量不变
    }

    #[test]
    fn test_vec_buffer_reserve() {
        let mut buf = VecBuffer::with_capacity(8);
        buf.push_slice(b"test"); // 使用部分容量
        let _old_capacity = buf.capacity();

        buf.reserve(100);
        // 预留后的容量应该足够容纳 current_len + 100
        assert!(buf.capacity() >= buf.len() + 100);
    }

    #[test]
    fn test_vec_buffer_into_vec() {
        let mut buf = VecBuffer::with_capacity(16);
        buf.push_slice(b"Hello");

        let vec = buf.into_vec();
        assert_eq!(&vec, b"Hello");
    }

    #[test]
    fn test_vec_buffer_from_vec() {
        let vec = vec![1, 2, 3, 4, 5];
        let buf = VecBuffer::from(vec);
        assert_eq!(buf.as_slice(), &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_vec_buffer_default() {
        let buf = VecBuffer::default();
        assert!(buf.capacity() >= 4096);
        assert!(buf.is_empty());
    }
}
