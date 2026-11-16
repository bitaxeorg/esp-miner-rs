//! Exposes an `embedded-io(-async)` 0.7 stream (e.g. `embassy_net::tcp::TcpSocket`)
//! through the 0.6 traits still required by `stratum-v1` 0.1.
//!
//! Remove once `stratum-v1` moves to `embedded-io-async` 0.7.

use embedded_io_06::ErrorKind;

/// `stratum-v1` maps every I/O error to `Error::NetworkError`, so reporting
/// them all as `ErrorKind::Other` loses nothing.
pub struct Io06<T>(pub T);

impl<T: embedded_io::ErrorType> embedded_io_06::ErrorType for Io06<T> {
    type Error = ErrorKind;
}

impl<T: embedded_io_async::Read> embedded_io_async_06::Read for Io06<T> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.read(buf).await.map_err(|_| ErrorKind::Other)
    }
}

impl<T: embedded_io::ReadReady> embedded_io_06::ReadReady for Io06<T> {
    fn read_ready(&mut self) -> Result<bool, Self::Error> {
        self.0.read_ready().map_err(|_| ErrorKind::Other)
    }
}

impl<T: embedded_io_async::Write> embedded_io_async_06::Write for Io06<T> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.0.write(buf).await.map_err(|_| ErrorKind::Other)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush().await.map_err(|_| ErrorKind::Other)
    }
}
