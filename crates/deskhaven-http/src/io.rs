//! hyper's header-read timer and the request deadline do not cover writing the response: a
//! client that never reads keeps hyper blocked in `poll_write` and pins a connection slot.
//! A pending write opens a window that must see `min_bytes_per_sec * window` bytes, or the
//! write fails with `TimedOut`. Reads are not timed, so idle keep-alive stays governed by
//! `header_read_timeout`.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::Sleep;

const MILLIS_PER_SEC: u128 = 1000;

struct Window {
    deadline: Pin<Box<Sleep>>,
    written: u64,
}

pub(crate) struct MinWriteRate<S> {
    inner: S,
    window: Duration,
    /// Bytes a window must see; 0 = any progress is enough.
    required: u64,
    open: Option<Window>,
}

impl<S> MinWriteRate<S> {
    pub(crate) fn new(inner: S, window: Duration, min_bytes_per_sec: u64) -> Self {
        let required = u128::from(min_bytes_per_sec) * window.as_millis() / MILLIS_PER_SEC;
        Self {
            inner,
            window,
            required: u64::try_from(required).unwrap_or(u64::MAX),
            open: None,
        }
    }

    fn progressed(&mut self, bytes: usize) {
        let Some(window) = self.open.as_mut() else {
            return;
        };
        window.written = window
            .written
            .saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
        if window.written >= self.required {
            self.open = None;
        }
    }

    fn pending<T>(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<T>> {
        let length = self.window;
        let window = self.open.get_or_insert_with(|| Window {
            deadline: Box::pin(tokio::time::sleep(length)),
            written: 0,
        });
        match window.deadline.as_mut().poll(cx) {
            Poll::Ready(()) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "peer reads the response too slowly",
            ))),
            Poll::Pending => Poll::Pending,
        }
    }

    fn track_write(
        &mut self,
        cx: &mut Context<'_>,
        res: Poll<io::Result<usize>>,
    ) -> Poll<io::Result<usize>> {
        match res {
            Poll::Ready(Ok(n)) => {
                self.progressed(n);
                Poll::Ready(Ok(n))
            }
            Poll::Ready(Err(err)) => Poll::Ready(Err(err)),
            Poll::Pending => self.pending(cx),
        }
    }

    fn track_other(
        &mut self,
        cx: &mut Context<'_>,
        res: Poll<io::Result<()>>,
    ) -> Poll<io::Result<()>> {
        match res {
            Poll::Pending => self.pending(cx),
            ready => ready,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for MinWriteRate<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for MinWriteRate<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let res = Pin::new(&mut this.inner).poll_write(cx, buf);
        this.track_write(cx, res)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let res = Pin::new(&mut this.inner).poll_write_vectored(cx, bufs);
        this.track_write(cx, res)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let res = Pin::new(&mut this.inner).poll_flush(cx);
        this.track_other(cx, res)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let res = Pin::new(&mut this.inner).poll_shutdown(cx);
        this.track_other(cx, res)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex};

    const WINDOW: Duration = Duration::from_millis(200);
    /// 1024 B/s over 200 ms -> 204 bytes per window.
    const MIN_RATE: u64 = 1024;
    const PIPE: usize = 64;
    const PAYLOAD: usize = 4 * 1024;

    async fn drip(mut far: DuplexStream, chunk: usize, every: Duration) -> usize {
        let mut buf = vec![0u8; chunk];
        let mut total = 0;
        loop {
            tokio::time::sleep(every).await;
            match far.read(&mut buf).await {
                Ok(0) | Err(_) => return total,
                Ok(n) => total += n,
            }
        }
    }

    async fn write_payload(near: DuplexStream) -> io::Result<()> {
        let mut stream = MinWriteRate::new(near, WINDOW, MIN_RATE);
        stream.write_all(&[0u8; PAYLOAD]).await?;
        stream.shutdown().await
    }

    #[tokio::test(start_paused = true)]
    async fn non_reader_times_out() {
        let (near, _far) = duplex(PIPE);
        let err = write_payload(near).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[tokio::test(start_paused = true)]
    async fn drip_reader_below_the_floor_times_out() {
        let (near, far) = duplex(PIPE);
        // 16 bytes / 50 ms = 320 B/s < 1024 B/s.
        let reader = tokio::spawn(drip(far, 16, Duration::from_millis(50)));
        let err = write_payload(near).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        let read = reader.await.unwrap();
        assert!(read > 0 && read < PAYLOAD, "reader got {read} bytes");
    }

    #[tokio::test(start_paused = true)]
    async fn reader_above_the_floor_is_unaffected() {
        let (near, far) = duplex(PIPE);
        // 64 bytes / 10 ms = 6400 B/s > 1024 B/s.
        let reader = tokio::spawn(drip(far, PIPE, Duration::from_millis(10)));
        write_payload(near).await.unwrap();
        assert_eq!(reader.await.unwrap(), PAYLOAD);
    }

    #[tokio::test]
    async fn fast_reader_is_unaffected() {
        let (near, mut far) = duplex(PIPE);
        let reader = tokio::spawn(async move {
            let mut buf = Vec::new();
            far.read_to_end(&mut buf).await.map(|_| buf.len())
        });
        write_payload(near).await.unwrap();
        assert_eq!(reader.await.unwrap().unwrap(), PAYLOAD);
    }

    #[tokio::test(start_paused = true)]
    async fn reads_are_not_timed() {
        let (near, mut far) = duplex(PIPE);
        let mut stream = MinWriteRate::new(near, WINDOW, MIN_RATE);
        let writer = tokio::spawn(async move {
            tokio::time::sleep(WINDOW * 3).await;
            far.write_all(b"late").await
        });
        let mut buf = [0u8; 4];
        stream.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"late");
        writer.await.unwrap().unwrap();
    }
}
