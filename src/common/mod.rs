use bitcode::{Encode, Decode};

pub mod shares;

#[derive(Encode, Decode, Clone, Debug)]
pub enum ClientMessage {
    Subscribe,
    Kill,
    Publish { message: String },
}

#[derive(Encode, Decode, Clone, Debug)]
pub enum ServerMessage {
    Shutdown,
    Message { message: String },
}
