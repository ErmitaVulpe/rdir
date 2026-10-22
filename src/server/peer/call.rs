use bitcode::{Decode, Encode};
use derive_more::{Display, Error, From, IsVariant};
use tokio::io;

use crate::{
    common::shares::Authority,
    server::{PeerId, state::SharedState},
};

pub async fn call_peer(authority: Authority, state: SharedState) -> Result<(), CallPeerError> {
    todo!()
}

#[derive(Encode, Decode, Clone, Debug, Display, Error, From, IsVariant)]
pub enum CallPeerError {
    #[display("Connection timed out")]
    Timeout,
    #[display("IO error: {_0}")]
    Io(
        #[error(ignore)]
        #[from(io::Error)]
        String,
    ),
}

// /// returns whether this stream spawned a long-lived task
// async fn handle_stream_new_peer(
//     stream: yamux::Stream,
//     peer_id: PeerId,
//     peer_socket: SocketAddrV4,
//     intent: PeerInitReq,
//     state: SharedState,
// ) -> io::Result<(Option<mpsc::Receiver<PeerNotification>>, PeerInitResp)> {
//     let mut stream = LengthDelimitedCodec::builder()
//         .length_field_length(3)
//         .new_framed(stream.compat());
//     stream
//         .send(encode(&intent.clone().into_dto()).into())
//         .await?;
//     debug!("Sent stream init request");
//
//     let resp_bytes = stream
//         .next()
//         .timeout(MESSAGE_TIMEOUT)
//         .await?
//         .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))??;
//     let resp: PeerInitResp =
//         decode(&resp_bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
//
//     match intent {
//         PeerInitReq::JoinShare { name, path } => match resp {
//             PeerInitResp::Ok => {
//                 let result = state.write().await.new_peer_join_remote_share(
//                     peer_id,
//                     peer_socket,
//                     name,
//                     path,
//                 );
//                 match result {
//                     Ok(rx) => {
//                         spawn(long_lived_share_client(stream));
//                         Ok((Some(rx), resp))
//                     }
//                     Err(_) => {
//                         // if this branch is reached, it basically means that
//                         // the peer lied to us
//                         Err(ProtocolError.into())
//                     }
//                 }
//             }
//             PeerInitResp::Err(_) => Ok((None, resp)),
//             _ => Err(ProtocolError.into()),
//         },
//         PeerInitReq::ListShares => todo!(),
//     }
// }

pub async fn handle_stream(
    /* stream: yamux::Stream, */
    peer_id: PeerId,
    state: SharedState,
) -> io::Result<()> {
    todo!()

    // let mut stream = LengthDelimitedCodec::builder()
    //     .length_field_length(3)
    //     .new_framed(stream.compat());
    // stream
    //     .send(encode(&intent.clone().into_dto()).into())
    //     .await?;
    // debug!("Sent stream init request");
    //
    // let resp_bytes = stream
    //     .next()
    //     .timeout(MESSAGE_TIMEOUT)
    //     .await?
    //     .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))??;
    // let resp: PeerInitResp =
    //     decode(&resp_bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    //
    // match intent {
    //     PeerInitReq::JoinShare { name, path } => match resp {
    //         PeerInitResp::Ok => {
    //             let result = state.write().await.join_remote_share(peer_id, name, path);
    //             match result {
    //                 Ok(()) => {
    //                     spawn(long_lived_share_client(stream));
    //                     Ok(resp)
    //                 }
    //                 Err(_) => {
    //                     // if this branch is reached, it basically means that
    //                     // the peer lied to us
    //                     Err(ProtocolError.into())
    //                 }
    //             }
    //         }
    //         PeerInitResp::Err(_) => Ok(resp),
    //         _ => Err(ProtocolError.into()),
    //     },
    //     PeerInitReq::ListShares => todo!(),
    // }
}
