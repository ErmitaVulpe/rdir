use std::{
    io::ErrorKind,
    net::{SocketAddr, SocketAddrV4},
    pin::Pin,
    sync::LazyLock,
    task::{Context, Poll, Waker},
    time::Duration,
};

use derive_more::{Display, Error, From, IsVariant};
use futures::ready;
use pin_project::pin_project;
use smol::{
    io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
};
use smol_timeout::TimeoutExt;
use snow::{Builder, HandshakeState, TransportState, params::NoiseParams};

pub const FRAMED_TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const FRAMED_TCP_TIMEOUT: Duration = Duration::from_secs(2);

static PARAMS: LazyLock<NoiseParams> =
    LazyLock::new(|| "Noise_NN_25519_AESGCM_BLAKE2b".parse().unwrap());

const LENGTH_FIELD_LEN: usize = std::mem::size_of::<u16>();
const TAG_LEN: usize = 16;
const MAX_MESSAGE_LEN: usize = u16::MAX as usize;

pub struct PeerConnection {
    inner: yamux::Connection<NoiseStream<TcpStream>>,
    peer_addr: SocketAddrV4,
}

impl PeerConnection {
    pub async fn connect(addr: SocketAddrV4) -> Result<Self, NoiseStreamError> {
        async {
            let stream = TcpStream::connect(addr).await?;
            let state = Builder::new(PARAMS.clone()).build_initiator()?;
            let noise_stream = NoiseStream::handshake(stream, state).await?;

            let SocketAddr::V4(peer_addr) = noise_stream.get_inner().peer_addr()? else {
                return Err(io::Error::from(io::ErrorKind::Unsupported).into());
            };
            let inner =
                yamux::Connection::new(noise_stream, Default::default(), yamux::Mode::Client);
            Ok(Self { inner, peer_addr })
        }
        .timeout(FRAMED_TCP_CONNECT_TIMEOUT)
        .await
        .ok_or(io::Error::from(io::ErrorKind::TimedOut))?
    }

    pub async fn accept(stream: TcpStream) -> Result<Self, NoiseStreamError> {
        async {
            let state = Builder::new(PARAMS.clone()).build_responder()?;
            let noise_stream = NoiseStream::handshake(stream, state).await?;

            let SocketAddr::V4(peer_addr) = noise_stream.get_inner().peer_addr()? else {
                return Err(io::Error::from(io::ErrorKind::Unsupported).into());
            };
            let inner =
                yamux::Connection::new(noise_stream, Default::default(), yamux::Mode::Server);
            Ok(Self { inner, peer_addr })
        }
        .timeout(FRAMED_TCP_CONNECT_TIMEOUT)
        .await
        .ok_or(io::Error::from(io::ErrorKind::TimedOut))?
    }

    pub async fn new_outbound(&mut self) -> yamux::Result<yamux::Stream> {
        
    }
}

#[derive(Debug)]
enum ReadState {
    ShuttingDown,
    Idle,
    ReadingLen(usize, [u8; 2]),
    ReadingMessage(usize),
    ServingPayload(usize),
}

#[derive(Debug)]
enum WriteState {
    ShuttingDown,
    Idle,
    WritingMessage(usize, usize),
}

#[pin_project]
pub struct NoiseStream<T> {
    #[pin]
    inner: T,

    transport: TransportState,
    read_state: ReadState,
    write_state: WriteState,
    write_clean_waker: Option<Waker>,

    read_message_buffer: Vec<u8>,
    read_payload_buffer: Vec<u8>,

    write_message_buffer: Vec<u8>,
}

impl<T> NoiseStream<T> {
    pub fn get_inner(&self) -> &T {
        &self.inner
    }

    pub fn get_inner_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    pub fn into_inner(self) -> T {
        self.inner
    }

    pub fn get_state(&self) -> &TransportState {
        &self.transport
    }

    pub fn get_state_mut(&mut self) -> &mut TransportState {
        &mut self.transport
    }
}

impl<T> NoiseStream<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    async fn handshake(mut stream: T, mut state: HandshakeState) -> Result<Self, NoiseStreamError> {
        loop {
            if state.is_handshake_finished() {
                let transport = state.into_transport_mode()?;
                return Ok(Self {
                    inner: stream,
                    transport,
                    read_state: ReadState::Idle,
                    write_state: WriteState::Idle,
                    write_clean_waker: None,
                    read_message_buffer: vec![0; MAX_MESSAGE_LEN],
                    read_payload_buffer: vec![0; MAX_MESSAGE_LEN],
                    write_message_buffer: vec![0; LENGTH_FIELD_LEN + MAX_MESSAGE_LEN],
                });
            }

            let mut message = vec![0; MAX_MESSAGE_LEN];
            let mut payload = vec![0; MAX_MESSAGE_LEN];

            if state.is_my_turn() {
                let len = state.write_message(&[], &mut message)?;
                let prefix = (len as u16).to_be_bytes();
                stream.write_all(&prefix).await?;
                stream.write_all(&message[..len]).await?;
                stream.flush().await?;
            } else {
                let mut len_buf = [0; 2];
                stream.read_exact(&mut len_buf).await?;
                let len = u16::from_be_bytes(len_buf) as usize;
                stream.read_exact(&mut message[..len]).await?;
                state.read_message(&message[..len], &mut payload)?;
            }
        }
    }
}

impl<T> AsyncWrite for NoiseStream<T>
where
    T: AsyncWrite,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        let this = self.project();
        let mut inner = this.inner;
        let state = this.write_state;
        let transport = this.transport;
        let write_message_buffer = this.write_message_buffer;

        loop {
            match state {
                WriteState::ShuttingDown => {
                    return Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()));
                }
                WriteState::Idle => {
                    let payload_len = buf.len().min(MAX_MESSAGE_LEN - TAG_LEN);
                    let buf = &buf[..payload_len];
                    write_message_buffer.resize(LENGTH_FIELD_LEN + MAX_MESSAGE_LEN, 0);

                    let message_len = transport
                        .write_message(buf, &mut write_message_buffer[LENGTH_FIELD_LEN..])
                        .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
                    write_message_buffer[..LENGTH_FIELD_LEN]
                        .copy_from_slice(&(message_len as u16).to_le_bytes());
                    write_message_buffer.truncate(LENGTH_FIELD_LEN + message_len);
                    *state = WriteState::WritingMessage(0, payload_len);
                }
                WriteState::WritingMessage(start, payload_len) => {
                    let n = ready!(
                        Pin::new(&mut inner).poll_write(cx, &write_message_buffer[*start..])
                    )?;
                    *start += n;

                    if *start == write_message_buffer.len() {
                        let n = *payload_len;
                        *state = WriteState::Idle;
                        if let Some(waker) = this.write_clean_waker.take() {
                            waker.wake();
                        }
                        return Poll::Ready(Ok(n));
                    }
                }
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        let this = self.project();
        match this.write_state {
            WriteState::ShuttingDown | WriteState::Idle => {
                return Poll::Ready(Ok(()));
            }
            _ => {}
        }

        *this.write_clean_waker = Some(cx.waker().clone());
        ready!(this.inner.poll_flush(cx))?;
        Poll::Pending
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        let this = self.project();
        if let Some(waker) = this.write_clean_waker.take() {
            waker.wake();
        }
        *this.write_state = WriteState::ShuttingDown;
        this.inner.poll_close(cx)
    }
}

impl<T> AsyncRead for NoiseStream<T>
where
    T: AsyncRead,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        out: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.project();

        let mut inner = this.inner;
        let state = this.read_state;
        let transport = this.transport;

        let read_message_buffer = this.read_message_buffer;
        let read_payload_buffer = this.read_payload_buffer;

        loop {
            match state {
                ReadState::ShuttingDown => {
                    return Poll::Ready(Ok(0));
                }

                ReadState::Idle => {
                    *state = ReadState::ReadingLen(0, [0; LENGTH_FIELD_LEN]);
                }

                ReadState::ReadingLen(read_len, buf) => {
                    if *read_len == LENGTH_FIELD_LEN {
                        let message_len = u16::from_le_bytes(*buf);
                        read_message_buffer.resize(message_len as usize, 0);
                        *state = ReadState::ReadingMessage(0);
                    } else {
                        let n = ready!(inner.as_mut().poll_read(cx, &mut buf[*read_len..],))?;

                        if n == 0 {
                            *state = ReadState::ShuttingDown;
                        } else {
                            *read_len += n;
                        }
                    }
                }

                ReadState::ReadingMessage(start) => {
                    if *start == read_message_buffer.len() {
                        read_payload_buffer.resize(MAX_MESSAGE_LEN, 0);
                        let n = transport
                            .read_message(read_message_buffer, read_payload_buffer)
                            .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;

                        read_payload_buffer.truncate(n);
                        *state = ReadState::ServingPayload(0);
                    } else {
                        let n = ready!(
                            inner
                                .as_mut()
                                .poll_read(cx, &mut read_message_buffer[*start..])
                        )?;

                        if n == 0 {
                            *state = ReadState::ShuttingDown;
                        } else {
                            *start += n;
                        }
                    }
                }

                ReadState::ServingPayload(start) => {
                    let available = read_payload_buffer.len() - *start;
                    let to_copy = available.min(out.len());

                    out[..to_copy].copy_from_slice(&read_payload_buffer[*start..*start + to_copy]);

                    if to_copy == available {
                        *state = ReadState::Idle;
                    } else {
                        *start += to_copy;
                    }

                    return Poll::Ready(Ok(to_copy));
                }
            }
        }
    }
}

#[derive(Debug, Display, Error, From, IsVariant)]
#[display("Error with Encrypted IO")]
pub enum NoiseStreamError {
    Io(io::Error),
    Crypto(snow::Error),
}

#[cfg(test)]
mod tests {
    use smol::{
        block_on,
        net::{TcpListener, TcpStream},
        spawn,
    };
    use snow::Builder;

    use super::*;

    #[test]
    fn tcp() {
        let result = async {
            static PATTERN: &str = "Noise_KK_25519_ChaChaPoly_BLAKE2s";
            let client_key = Builder::new(PATTERN.parse().unwrap())
                .generate_keypair()
                .unwrap();
            let server_key = Builder::new(PATTERN.parse().unwrap())
                .generate_keypair()
                .unwrap();

            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let task = spawn(async move {
                let initiator = Builder::new(PATTERN.parse().unwrap())
                    .local_private_key(&client_key.private)
                    .unwrap()
                    .remote_public_key(&server_key.public)
                    .unwrap()
                    .build_initiator()
                    .unwrap();
                let stream = TcpStream::connect(addr).await.unwrap();
                let mut stream = NoiseStream::handshake(stream, initiator).await.unwrap();
                let payload = (0..0x20000).map(|a| a as u8).collect::<Vec<_>>();
                stream.write_all(&payload).await.unwrap();
            });

            let responder = Builder::new(PATTERN.parse().unwrap())
                .local_private_key(&server_key.private)
                .unwrap()
                .remote_public_key(&client_key.public)
                .unwrap()
                .build_responder()
                .unwrap();
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = NoiseStream::handshake(stream, responder).await.unwrap();
            let mut payload = vec![0; 0x20000];
            stream.read_exact(&mut payload).await.unwrap();

            payload.iter().enumerate().for_each(|(i, v)| {
                assert_eq!(i as u8, *v);
            });

            task.cancel().await;
            anyhow::Ok(())
        };
        block_on(result).unwrap();
    }

    #[test]
    fn tcp_read_twice() {
        let result = async {
            static PATTERN: &str = "Noise_KK_25519_ChaChaPoly_BLAKE2s";
            let client_key = Builder::new(PATTERN.parse().unwrap())
                .generate_keypair()
                .unwrap();
            let server_key = Builder::new(PATTERN.parse().unwrap())
                .generate_keypair()
                .unwrap();

            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let task = spawn(async move {
                let initiator = Builder::new(PATTERN.parse().unwrap())
                    .local_private_key(&client_key.private)
                    .unwrap()
                    .remote_public_key(&server_key.public)
                    .unwrap()
                    .build_initiator()
                    .unwrap();
                let stream = TcpStream::connect(addr).await.unwrap();
                let mut stream = NoiseStream::handshake(stream, initiator).await.unwrap();
                let payload = (0..0x20000).map(|a| a as u8).collect::<Vec<_>>();
                stream.write_all(&payload).await.unwrap();
            });

            let responder = Builder::new(PATTERN.parse().unwrap())
                .local_private_key(&server_key.private)
                .unwrap()
                .remote_public_key(&client_key.public)
                .unwrap()
                .build_responder()
                .unwrap();
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = NoiseStream::handshake(stream, responder).await.unwrap();
            let mut payload1 = vec![0; 0x10000];
            stream.read_exact(&mut payload1).await.unwrap();
            let mut payload2 = vec![0; 0x10000];
            stream.read_exact(&mut payload2).await.unwrap();

            payload1
                .iter()
                .chain(payload2.iter())
                .enumerate()
                .for_each(|(i, v)| {
                    assert_eq!(i as u8, *v);
                });

            task.cancel().await;
            anyhow::Ok(())
        };
        block_on(result).unwrap();
    }

    #[test]
    fn snow() -> Result<(), Box<dyn std::error::Error>> {
        static PATTERN: &str = "Noise_NN_25519_ChaChaPoly_BLAKE2s";
        let mut initiator = snow::Builder::new(PATTERN.parse()?).build_initiator()?;
        let mut responder = snow::Builder::new(PATTERN.parse()?).build_responder()?;

        let (mut read_buf, mut first_msg, mut second_msg) = ([0u8; 1024], [0u8; 1024], [0u8; 1024]);

        // -> e
        let len = initiator.write_message(&[], &mut first_msg)?;

        // responder processes the first message...
        responder.read_message(&first_msg[..len], &mut read_buf)?;

        println!("first {:?}", &first_msg[..len]);

        // <- e, ee
        let len = responder.write_message(&[], &mut second_msg)?;

        println!("second {:?}", &second_msg[..len]);

        // initiator processes the response...
        initiator.read_message(&second_msg[..len], &mut read_buf)?;

        // NN handshake complete, transition into transport mode.
        let _initiator = initiator.into_transport_mode().unwrap();
        let _responder = responder.into_transport_mode().unwrap();
        Ok(())
    }
}
