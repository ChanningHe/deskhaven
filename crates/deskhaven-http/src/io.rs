//! Write-stall timeout for accepted connections (review H-a).
//!
//! hyper's header-read timer and the request deadline do not cover writing the
//! response: a client that never reads keeps hyper blocked in `poll_write`
//! forever and pins a connection slot. This wrapper fails a write (or flush /
//! shutdown) that has made no progress for `timeout`, which makes hyper close
//! the connection. The timer only runs while a write is pending, so idle
//! keep-alive connections stay governed by `header_read_timeout`.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::Sleep;

pub(crate) struct WriteStallTimeout<S> {
    inner: S,
    timeout: Duration,
    /// Armed by the first pending write, cleared by any write progress.
    stall: Option<Pin<Box<Sleep>>>,
}

impl<S> WriteStallTimeout<S> {
    pub(crate) fn new(inner: S, timeout: Duration) -> Self {
        Self {
            inner,
            timeout,
            stall: None,
        }
    }

    fn track<T>(&mut self, cx: &mut Context<'_>, res: Poll<io::Result<T>>) -> Poll<io::Result<T>> {
        if res.is_ready() {
            self.stall = None;
            return res;
        }
        let timeout = self.timeout;
        let stall = self
            .stall
            .get_or_insert_with(|| Box::pin(tokio::time::sleep(timeout)));
        match stall.as_mut().poll(cx) {
            Poll::Ready(()) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "peer stopped reading the response",
            ))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for WriteStallTimeout<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for WriteStallTimeout<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let res = Pin::new(&mut this.inner).poll_write(cx, buf);
        this.track(cx, res)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let res = Pin::new(&mut this.inner).poll_write_vectored(cx, bufs);
        this.track(cx, res)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let res = Pin::new(&mut this.inner).poll_flush(cx);
        this.track(cx, res)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let res = Pin::new(&mut this.inner).poll_shutdown(cx);
        this.track(cx, res)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

    const STALL: Duration = Duration::from_millis(50);

    #[tokio::test]
    async fn stalled_write_fails_with_timed_out() {
        let (near, _far) = duplex(16);
        let mut stream = WriteStallTimeout::new(near, STALL);
        let err = stream.write_all(&[0u8; 1024]).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[tokio::test]
    async fn steady_progress_never_times_out() {
        let (near, mut far) = duplex(16);
        let reader = tokio::spawn(async move {
            let mut buf = Vec::new();
            far.read_to_end(&mut buf).await.map(|_| buf.len())
        });
        let mut stream = WriteStallTimeout::new(near, Duration::from_secs(5));
        stream.write_all(&[0u8; 64 * 1024]).await.unwrap();
        stream.shutdown().await.unwrap();
        drop(stream);
        assert_eq!(reader.await.unwrap().unwrap(), 64 * 1024);
    }

    #[tokio::test]
    async fn reads_are_not_timed() {
        let (near, mut far) = duplex(16);
        let mut stream = WriteStallTimeout::new(near, STALL);
        let writer = tokio::spawn(async move {
            tokio::time::sleep(STALL * 4).await;
            far.write_all(b"late").await
        });
        let mut buf = [0u8; 4];
        stream.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"late");
        writer.await.unwrap().unwrap();
    }
}
