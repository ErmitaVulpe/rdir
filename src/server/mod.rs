use derive_more::{Display, Error, From, IsVariant};

use crate::args::Args;

mod setup;
pub mod state;

pub const DOWNLOAD_CACHE_DIR: &str = "cache";
pub const LOGS_DIR: &str = "logs";
pub const LOGS_PREFIX: &str = "rdir.log";
pub const SOCKET_NAME: &str = "rdir.sock";
/// 29284
pub const NETWORK_PORT: u16 = u16::from_be_bytes(*b"rd");

pub fn run(args: Args, std_listener: std::os::unix::net::UnixListener) -> anyhow::Result<()> {
    let _guard = unsafe { setup::setup(&args) }?;

    setup::clean_up(&args);
    Ok(())
}

#[derive(Debug, Display, Error, From, IsVariant)]
#[display("Server encountered an error while processing the command")]
pub enum ServerError {
    // #[display("Specified share name is invalid")]
    // CommonShareNameParse(CommonShareNameParseError),
    // ConnectToRemoteShare(ConnectToRemoteShareError),
    InvalidShareName,
    // PeerIo(NoiseStreamError),
    // RepeatedShare(RepeatedShare),
    // ShareDoesntExit(ShareDoesntExistError),
}
