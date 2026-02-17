use std::{
    net::{SocketAddr, SocketAddrV4},
    sync::LazyLock,
    time::Duration,
};

use derive_more::{Display, Error, From, IsVariant};
use smol::{
    io::{self, AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use smol_timeout::TimeoutExt;
use snow::{Builder, TransportState, params::NoiseParams};
use tracing::debug;

pub const FRAMED_TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const FRAMED_TCP_TIMEOUT: Duration = Duration::from_secs(2);

static PARAMS: LazyLock<NoiseParams> =
    LazyLock::new(|| "Noise_NN_25519_AESGCM_BLAKE2b".parse().unwrap());

pub struct FramedTcpStream {
    stream: TcpStream,
    noise: TransportState,
    read_buf: Vec<u8>,
    write_buf: Vec<u8>,
    plain_buf: Vec<u8>,
}

impl FramedTcpStream {
    pub async fn new_initiator(addr: SocketAddrV4) -> Result<Self, FramedError> {
        debug!("Starting a new peer connection as initiator");
        let mut stream = TcpStream::connect(addr)
            .timeout(FRAMED_TCP_CONNECT_TIMEOUT)
            .await
            .ok_or(io::Error::from(io::ErrorKind::TimedOut))??;
        let mut initiator = Builder::new(PARAMS.clone()).build_initiator()?;
        let mut read_buf = vec![0u8; 1024];
        let mut write_buf = vec![0u8; 1024];
        let mut plain_buf = vec![0u8; 1024];

        let len = initiator.write_message(&[], &mut write_buf)?;
        Self::send(&mut stream, &write_buf[..len]).await?;

        Self::recv(&mut stream, &mut read_buf).await?;
        initiator.read_message(&read_buf, &mut plain_buf)?;

        debug!("Established connection as initiator");
        Ok(Self {
            stream,
            noise: initiator.into_transport_mode()?,
            read_buf,
            write_buf,
            plain_buf,
        })
    }

    pub async fn new_responder(mut stream: TcpStream) -> Result<Self, FramedError> {
        debug!("Starting a new peer connection as responder");
        let mut responder = Builder::new(PARAMS.clone()).build_responder()?;
        let mut read_buf = vec![0u8; 1024];
        let mut write_buf = vec![0u8; 1024];
        let mut plain_buf = vec![0u8; 1024];

        Self::recv(&mut stream, &mut read_buf).await?;
        responder.read_message(&read_buf, &mut plain_buf)?;

        let len = responder.write_message(&[], &mut write_buf)?;
        Self::send(&mut stream, &write_buf[..len]).await?;

        debug!("Established connection as responder");
        Ok(Self {
            stream,
            noise: responder.into_transport_mode()?,
            read_buf,
            write_buf,
            plain_buf,
        })
    }

    /// `buf` will be resized to fit the entire frame
    async fn recv(stream: &mut TcpStream, buf: &mut Vec<u8>) -> io::Result<()> {
        let mut len_buf = [0u8; 2];
        stream.read_exact(&mut len_buf).await?;
        let len = u16::from_be_bytes(len_buf) as usize;
        buf.resize(len, 0);
        stream.read_exact(buf).await?;
        Ok(())
    }

    /// Writes the entire passed buffer
    async fn send(stream: &mut TcpStream, buf: &[u8]) -> io::Result<()> {
        let len = u16::try_from(buf.len())
            .expect("Tried to write a chunk that is bigger than 2^16 bytes");
        stream.write_all(&len.to_be_bytes()).await?;
        stream.write_all(buf).await
    }

    pub async fn read(&mut self) -> Result<Vec<u8>, FramedError> {
        Self::recv(&mut self.stream, &mut self.read_buf).await?;
        self.plain_buf.resize(self.read_buf.len(), 0);
        let n = self
            .noise
            .read_message(&self.read_buf, &mut self.plain_buf)?;

        Ok(self.plain_buf[..n].to_vec())
    }

    pub async fn write(&mut self, data: &[u8]) -> Result<(), FramedError> {
        self.write_buf.resize(data.len() + 16, 0);
        let len = self
            .noise
            .write_message(data, self.write_buf.as_mut_slice())?;
        Self::send(&mut self.stream, &self.write_buf[..len]).await?;
        Ok(())
    }

    pub async fn read_timeout(&mut self) -> Result<Vec<u8>, FramedError> {
        self.read()
            .timeout(FRAMED_TCP_TIMEOUT)
            .await
            .ok_or(io::Error::from(io::ErrorKind::TimedOut))?
    }

    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.stream.peer_addr()
    }
}

#[derive(Debug, Display, Error, From, IsVariant)]
#[display("Error with Encrypted IO")]
pub enum FramedError {
    Io(io::Error),
    Crypto(snow::Error),
}
