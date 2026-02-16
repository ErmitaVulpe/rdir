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
use snow::{Builder, HandshakeState, TransportState, params::NoiseParams};

pub const FRAMED_TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

static PARAMS: LazyLock<NoiseParams> =
    LazyLock::new(|| "Noise_XX_25519_AESGCM_SHA256".parse().unwrap());

pub struct FramedTcpStream {
    stream: TcpStream,
    noise: TransportState,
    read_buf: Vec<u8>,
    write_buf: Vec<u8>,
    plain_buf: Vec<u8>,
}

impl FramedTcpStream {
    pub async fn new_initiator(addr: SocketAddrV4) -> Result<Self, FramedError> {
        let stream = TcpStream::connect(addr)
            .timeout(FRAMED_TCP_CONNECT_TIMEOUT)
            .await
            .ok_or(io::Error::from(io::ErrorKind::TimedOut))??;
        let handshake = Builder::new(PARAMS.clone()).build_initiator()?;
        Self::new_inner(stream, handshake).await
    }

    pub async fn new_responder(stream: TcpStream) -> Result<Self, FramedError> {
        let handshake = Builder::new(PARAMS.clone()).build_initiator()?;
        Self::new_inner(stream, handshake).await
    }

    async fn new_inner(
        mut stream: TcpStream,
        mut handshake: HandshakeState,
    ) -> Result<Self, FramedError> {
        let mut read_buf = Vec::new();
        let mut write_buf = Vec::new();
        while !handshake.is_handshake_finished() {
            if handshake.is_initiator() {
                let len = handshake.write_message(&[], write_buf.as_mut_slice())?;
                stream.write_all(&write_buf[..len]).await?;
            }

            let n = stream.read(read_buf.as_mut_slice()).await?;
            handshake.read_message(&read_buf[..n], &mut [])?;
        }

        let noise = handshake.into_transport_mode()?;
        Ok(Self {
            stream,
            noise,
            read_buf,
            write_buf,
            plain_buf: Default::default(),
        })
    }

    pub async fn write(&mut self, data: &[u8]) -> Result<(), FramedError> {
        self.write_buf.resize(data.len() + 16, 0);
        let len = self
            .noise
            .write_message(data, self.write_buf.as_mut_slice())?;
        let len_prefix = (len as u16).to_be_bytes();

        self.stream.write_all(&len_prefix).await?;
        self.stream.write_all(&self.write_buf[..len]).await?;
        Ok(())
    }

    pub async fn read(&mut self) -> Result<Vec<u8>, FramedError> {
        let mut len_buf = [0u8; 2];
        self.stream.read_exact(&mut len_buf).await?;
        let len = u16::from_be_bytes(len_buf) as usize;

        self.read_buf.resize(len, 0);
        self.stream.read_exact(&mut self.read_buf).await?;

        self.plain_buf.resize(len, 0);
        let n = self
            .noise
            .read_message(&self.read_buf, &mut self.plain_buf)?;

        Ok(self.plain_buf[..n].to_vec())
    }

    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.stream.peer_addr()
    }
}

#[derive(Debug, Display, Error, From, IsVariant)]
pub enum FramedError {
    Io(io::Error),
    Crypto(snow::Error),
}
