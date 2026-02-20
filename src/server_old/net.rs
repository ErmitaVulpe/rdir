use derive_more::{Display, Error, From, IsVariant};
use smol::io;
use libp2p::{
    identity,
    noise,
    tcp,
    yamux,
    Transport,
};
use futures::prelude::*;

fn build_transport(
    keypair: &identity::Keypair,
) -> libp2p::core::transport::Boxed<(libp2p::PeerId, libp2p::core::muxing::StreamMuxerBox)> {

    let noise_config = noise::Config::new(keypair).unwrap();

    tcp::Transport::<smol::net::TcpStream>::new(tcp::Config::default())
        .upgrade(libp2p::core::upgrade::Version::V1)
        .authenticate(noise_config)
        .multiplex(yamux::Config::default())
        .boxed()
}

#[derive(Debug, Display, Error, From, IsVariant)]
#[display("Error with Encrypted IO")]
pub enum NoiseStreamError {
    Io(io::Error),
    Crypto(snow::Error),
}
