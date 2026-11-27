use std::{io, net::SocketAddr, time::Duration};

use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpStream,
    time::timeout,
};

/// Flag that makes a process binary probe its own readiness and exit.
pub const PROBE_READINESS_FLAG: &str = "--probe-readiness";

const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST: &[u8] = b"GET /readyz HTTP/1.1\r\nhost: localhost\r\nconnection: close\r\n\r\n";
const READY_STATUS_LINE: &[u8] = b"HTTP/1.1 200";

/// Returns whether `GET /readyz` on `address` answers 200.
///
/// Distroless images ship no shell or curl, so a compose healthcheck runs the
/// product binary itself with [`PROBE_READINESS_FLAG`].
///
/// # Errors
///
/// Returns an I/O error when the connection fails or times out.
pub async fn probe_readiness(address: SocketAddr) -> io::Result<bool> {
    timeout(PROBE_TIMEOUT, async {
        let mut stream = TcpStream::connect(address).await?;
        stream.write_all(REQUEST).await?;
        let mut status = [0_u8; READY_STATUS_LINE.len()];
        stream.read_exact(&mut status).await?;
        Ok(status == READY_STATUS_LINE)
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "readiness probe timed out"))?
}

#[cfg(test)]
mod tests {
    use tokio::net::TcpListener;

    use super::*;

    async fn answer(response: &'static [u8]) -> io::Result<SocketAddr> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut request = [0_u8; REQUEST.len()];
                let _ = stream.read_exact(&mut request).await;
                let _ = stream.write_all(response).await;
            }
        });
        Ok(address)
    }

    #[tokio::test]
    async fn ready_when_readyz_answers_200() -> io::Result<()> {
        let address = answer(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n").await?;
        assert!(probe_readiness(address).await?);
        Ok(())
    }

    #[tokio::test]
    async fn not_ready_when_readyz_answers_503() -> io::Result<()> {
        let address =
            answer(b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\n\r\n").await?;
        assert!(!probe_readiness(address).await?);
        Ok(())
    }
}
