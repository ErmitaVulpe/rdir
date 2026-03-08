use std::{
    hint::unreachable_unchecked,
    pin::Pin,
    task::{Poll, ready},
};

use snowstorm::NoiseStream;
use tokio::{
    io,
    net::TcpStream,
    sync::{mpsc, oneshot},
};
use tokio_util::compat::Compat;
use tracing::error;

use crate::server::{
    PeerId,
    peer::PeerInitResp,
    state::{PeerNotification, SharedState},
};

pub struct ConnDriver {
    command_rx: mpsc::Receiver<PeerNotification>,
    conn: yamux::Connection<Compat<NoiseStream<TcpStream>>>,
    conn_state: ConnState,
    peer_id: PeerId,
    state: SharedState,
}

impl ConnDriver {
    pub fn new(
        conn: yamux::Connection<Compat<NoiseStream<TcpStream>>>,
        peer_id: PeerId,
        command_rx: mpsc::Receiver<PeerNotification>,
        state: SharedState,
    ) -> Self {
        Self {
            command_rx,
            conn,
            conn_state: ConnState::Normal,
            peer_id,
            state,
        }
    }
}

impl Future for ConnDriver {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        loop {
            match &mut self.conn_state {
                ConnState::HandlingOutboundStream { fut, .. } => {
                    let resp = match ready!(fut.as_mut().poll(cx)) {
                        Ok(resp) => resp,
                        Err(err) => {
                            error!(
                                "Error during handshake of a new outbound stream with peer {}: {err}",
                                self.peer_id
                            );
                            break Poll::Ready(());
                        }
                    };

                    // Same disgusting trick as in ConnState::OpeningNewOutBoundStream
                    let conn_state = std::mem::replace(&mut self.conn_state, ConnState::Normal);
                    let resp_tx = match conn_state {
                        ConnState::HandlingOutboundStream { resp_tx, .. } => resp_tx,
                        _ => unsafe { unreachable_unchecked() },
                    };

                    if let Some(resp_tx) = resp_tx {
                        let _ = resp_tx.send(resp);
                    }
                }
                ConnState::Normal => {
                    // check for incoming messages first
                    match self.command_rx.poll_recv(cx) {
                        Poll::Ready(Some(notification)) => {
                            self.conn_state = ConnState::OpeningNewOutBoundStream(notification);
                        }
                        Poll::Ready(None) => self.conn_state = ConnState::Shutdown,
                        Poll::Pending => {
                            let new_stream = ready!(self.conn.poll_next_inbound(cx))
                                .expect("The underlying connection is already closed");
                            match new_stream {
                                Ok(stream) => stream,
                                Err(err) => {
                                    error!(
                                        "Error while accepting a new inbound stream with peer {}: {err}",
                                        self.peer_id
                                    );
                                    break Poll::Ready(());
                                }
                            };
                        }
                    }
                }
                ConnState::OpeningNewOutBoundStream(_) => {
                    let stream = match self.conn.poll_new_outbound(cx) {
                        Poll::Ready(Ok(stream)) => stream,
                        Poll::Ready(Err(err)) => {
                            error!(
                                "Error while opening a new outbound stream with peer {}: {err}",
                                self.peer_id
                            );
                            break Poll::Ready(());
                        }
                        Poll::Pending => break Poll::Pending,
                    };

                    // Sadly this is necessary since the borrow checker doesnt differentiate
                    // borrows of sparate fields on a single struct. All I do here is temporarily
                    // replace current conn_state with a dummy to get owned notification
                    let conn_state = std::mem::replace(&mut self.conn_state, ConnState::Normal);
                    let PeerNotification { req, resp } = match conn_state {
                        ConnState::OpeningNewOutBoundStream(peer_notification) => peer_notification,
                        _ => unsafe { unreachable_unchecked() },
                    };

                    let fut = super::call::handle_stream(
                        stream,
                        self.peer_id.clone(),
                        req.clone(),
                        self.state.clone(),
                    );

                    self.conn_state = ConnState::HandlingOutboundStream {
                        fut: Box::pin(fut),
                        resp_tx: resp,
                    };
                }
                ConnState::Shutdown => {
                    if let Err(err) = ready!(self.conn.poll_close(cx)) {
                        error!(
                            "Error while closing the connection with peer {}: {err}",
                            self.peer_id
                        );
                    }
                    break Poll::Ready(());
                }
            }
        }
    }
}

enum ConnState {
    HandlingOutboundStream {
        fut: Pin<Box<dyn Future<Output = io::Result<PeerInitResp>> + Send + Sync>>,
        resp_tx: Option<oneshot::Sender<PeerInitResp>>,
    },
    Normal,
    OpeningNewOutBoundStream(PeerNotification),
    Shutdown,
}
