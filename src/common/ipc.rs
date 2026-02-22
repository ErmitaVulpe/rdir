use std::marker::PhantomData;

use bitcode::{decode, encode};
use futures::{SinkExt, StreamExt};
use tokio::{io, net::UnixStream};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use crate::common::{ClientMessage, ServerResponse};

pub struct IpcStream<Side> {
    inner: Framed<UnixStream, LengthDelimitedCodec>,
    _marker: PhantomData<Side>,
}

/// Maker indicating the side of `IpcStream`
pub struct Client;
/// Maker indicating the side of `IpcStream`
pub struct Server;

impl IpcStream<Client> {
    pub fn new_client(stream: UnixStream) -> Self {
        Self {
            inner: length_delimited(stream),
            _marker: PhantomData,
        }
    }

    pub async fn send_command(&mut self, command: &ClientMessage) -> io::Result<ServerResponse> {
        let encoded = encode(command);
        self.inner.send(encoded.into()).await?;
        let received = self
            .inner
            .next()
            .await
            .ok_or(io::Error::from(io::ErrorKind::UnexpectedEof))??;
        decode(&received).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

impl IpcStream<Server> {
    pub fn new_server(stream: UnixStream) -> Self {
        Self {
            inner: length_delimited(stream),
            _marker: PhantomData,
        }
    }

    pub async fn read_command(&mut self) -> io::Result<ClientMessage> {
        let received = self
            .inner
            .next()
            .await
            .ok_or(io::Error::from(io::ErrorKind::UnexpectedEof))??;
        decode(&received).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    pub async fn write_respone(&mut self, response: &ServerResponse) -> io::Result<()> {
        let encoded = encode(response);
        self.inner.send(encoded.into()).await
    }
}

fn length_delimited(stream: UnixStream) -> Framed<UnixStream, LengthDelimitedCodec> {
    LengthDelimitedCodec::builder()
        .length_field_type::<u16>()
        .new_framed(stream)
}
