use std::{
    net::{AddrParseError, Ipv4Addr},
    str::FromStr,
};

use bitcode::{Decode, Encode};
use derive_more::{Display, Error, From, IsVariant};

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
#[display("{addr}:{name}")]
pub struct FullShareName {
    addr: Ipv4Addr,
    name: CommonShareName,
}

impl FromStr for FullShareName {
    type Err = FullShareNameParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (raw_addr, raw_common_name) = s.split_once(':').ok_or(Self::Err::NoSeparator)?;
        let addr = Ipv4Addr::from_str(raw_addr)?;
        let name = CommonShareName::from_str(raw_common_name)?;
        Ok(FullShareName { addr, name })
    }
}

#[derive(Clone, Debug, Display, Error, From, IsVariant, PartialEq, Eq)]
pub enum FullShareNameParseError {
    #[display("{_0}")]
    InvalidAddress(AddrParseError),
    #[display("{_0}")]
    InvalidCommonShareName(CommonShareNameParseError),
    #[display("Full share name requires a separator \":\"")]
    NoSeparator,
}

#[derive(Encode, Decode, Clone, Debug, Display, PartialEq, Eq, PartialOrd, Ord)]
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

#[derive(Clone, Debug, Display, Error, IsVariant, PartialEq, Eq)]
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
        assert!(FullShareName::from_str("1.1.1.1:Example").is_ok());
        assert!(
            FullShareName::from_str("Example")
                .unwrap_err()
                .is_no_separator()
        );
        assert!(FullShareName::from_str("").unwrap_err().is_no_separator());
        assert!(
            FullShareName::from_str("Invalid IP:Example")
                .unwrap_err()
                .is_invalid_address()
        );
        assert!(
            FullShareName::from_str(&format!(
                "1.1.1.1:{}",
                "A".repeat(MAX_SHARE_NAME_LENGTH + 1)
            ))
            .unwrap_err()
            .is_invalid_common_share_name()
        );
    }

    #[test]
    fn share_name_parse() {
        assert!(ShareName::from_str("Example").unwrap().is_common());
        assert!(ShareName::from_str("1.1.1.1:Example").unwrap().is_full());
    }
}
