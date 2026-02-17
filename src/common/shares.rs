use std::{
    net::{AddrParseError, Ipv4Addr, SocketAddrV4},
    str::FromStr,
};

use bitcode::{Decode, Encode};
use derive_more::{AsRef, Display, Error, From, IsVariant};

use crate::server::NETWORK_PORT;

pub const MAX_SHARE_NAME_LENGTH: usize = 60;

#[derive(
    Encode, Decode, Clone, Debug, Display, From, IsVariant, PartialEq, Eq, PartialOrd, Ord,
)]
pub enum ShareName {
    Common(CommonShareName),
    Full(FullShareName),
}

impl FromStr for ShareName {
    type Err = ShareNameParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let full_err = match FullShareName::from_str(s) {
            Ok(val) => return Ok(val.into()),
            Err(e) => e,
        };
        if let Ok(val) = CommonShareName::from_str(s) {
            return Ok(val.into());
        }
        Err(full_err.into())
    }
}

#[derive(Clone, Debug, Display, Error, From, IsVariant, PartialEq, Eq)]
pub enum ShareNameParseError {
    #[display("Failed to parse the address as either common or full share name")]
    FailedToParseAsAny(#[error(source)] FullShareNameParseError),
}

#[derive(Encode, Decode, Clone, Debug, Display, PartialEq, Eq, PartialOrd, Ord)]
#[display("{addr}/{name}")]
pub struct FullShareName {
    pub addr: RemotePeerAddr,
    pub name: CommonShareName,
}

impl FromStr for FullShareName {
    type Err = FullShareNameParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (raw_addr, raw_common_name) = s.split_once('/').ok_or(Self::Err::NoSeparator)?;
        let addr = RemotePeerAddr::from_str(raw_addr)?;
        let name = CommonShareName::from_str(raw_common_name)?;
        Ok(FullShareName { addr, name })
    }
}

#[derive(Clone, Debug, Display, Error, From, IsVariant, PartialEq, Eq)]
pub enum FullShareNameParseError {
    #[display("{_0}")]
    InvalidAddress(#[error(source)] RemotePeerAddrParseError),
    #[display("Failed to parse the common share name")]
    InvalidCommonShareName(#[error(source)] CommonShareNameParseError),
    #[display("Full share name requires a separator \"/\" between address and name")]
    NoSeparator,
}

#[derive(Encode, Decode, Clone, Debug, Display, PartialEq, Eq, PartialOrd, Ord)]
#[display("{addr}{}", port.as_ref()
    .map(|p| format!(":{}", p))
    .unwrap_or_default()
)]
pub struct RemotePeerAddr {
    addr: Ipv4Addr,
    port: Option<u16>,
}

impl FromStr for RemotePeerAddr {
    type Err = RemotePeerAddrParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // TODO add sqids support
        match s.split_once(':') {
            Some((addr, port)) => Ok(Self {
                addr: addr.parse()?,
                port: {
                    let port: u16 = port
                        .parse()
                        .map_err(|_| RemotePeerAddrParseError::PortNumber(port.to_string()))?;
                    if port == NETWORK_PORT {
                        None
                    } else {
                        Some(port)
                    }
                },
            }),
            None => Ok(Self {
                addr: s.parse()?,
                port: None,
            }),
        }
    }
}

#[derive(Clone, Debug, Display, Error, From, IsVariant, PartialEq, Eq)]
pub enum RemotePeerAddrParseError {
    #[display("Failed to parse the ip address")]
    InvalidAddress(#[error(source)] AddrParseError),
    #[display("Could not parse \"{_0}\" as port")]
    PortNumber(#[error(ignore)] String),
}

impl From<RemotePeerAddr> for SocketAddrV4 {
    fn from(val: RemotePeerAddr) -> Self {
        SocketAddrV4::new(val.addr, val.port.unwrap_or(NETWORK_PORT))
    }
}

impl From<&RemotePeerAddr> for SocketAddrV4 {
    fn from(val: &RemotePeerAddr) -> Self {
        SocketAddrV4::new(val.addr, val.port.unwrap_or(NETWORK_PORT))
    }
}

#[derive(Encode, Decode, AsRef, Clone, Debug, Display, PartialEq, Eq, PartialOrd, Ord)]
pub struct CommonShareName(String);

impl FromStr for CommonShareName {
    type Err = CommonShareNameParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() > MAX_SHARE_NAME_LENGTH {
            return Err(Self::Err::NameTooLong);
        }

        Ok(Self(s.to_string()))
    }
}

#[derive(Encode, Decode, Clone, Debug, Display, Error, IsVariant, PartialEq, Eq)]
pub enum CommonShareNameParseError {
    #[display("Name of a share cannot exceed {MAX_SHARE_NAME_LENGTH} characters")]
    NameTooLong,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_share_name_parse() {
        assert!(CommonShareName::from_str("Example").is_ok());
        assert!(CommonShareName::from_str(&"A".repeat(MAX_SHARE_NAME_LENGTH)).is_ok());
        assert!(
            CommonShareName::from_str(&"A".repeat(MAX_SHARE_NAME_LENGTH + 1))
                .unwrap_err()
                .is_name_too_long()
        );
    }

    #[test]
    fn full_share_name_parse() {
        let name = FullShareName::from_str("1.2.3.4/Example").unwrap();
        assert_eq!(name.addr.addr, Ipv4Addr::from_octets([1, 2, 3, 4]));
        assert_eq!(name.addr.port, None);

        let name = FullShareName::from_str("1.2.3.4:1234/Example").unwrap();
        assert_eq!(name.addr.addr, Ipv4Addr::from_octets([1, 2, 3, 4]));
        assert_eq!(name.addr.port, Some(1234));

        let name = FullShareName::from_str(&format!("1.2.3.4:{NETWORK_PORT}/Example")).unwrap();
        assert_eq!(name.addr.addr, Ipv4Addr::from_octets([1, 2, 3, 4]));
        assert_eq!(name.addr.port, None);

        assert!(
            FullShareName::from_str("Example")
                .unwrap_err()
                .is_no_separator()
        );
        assert!(FullShareName::from_str("").unwrap_err().is_no_separator());
        assert!(
            FullShareName::from_str("Invalid IP/Example")
                .unwrap_err()
                .is_invalid_address()
        );
        assert!(
            FullShareName::from_str(&format!(
                "1.1.1.1/{}",
                "A".repeat(MAX_SHARE_NAME_LENGTH + 1)
            ))
            .unwrap_err()
            .is_invalid_common_share_name()
        );
    }

    #[test]
    fn share_name_parse() {
        assert!(ShareName::from_str("Example").unwrap().is_common());
        assert!(ShareName::from_str("1.1.1.1/Example").unwrap().is_full());
    }
}
