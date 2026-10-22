use std::{
    fmt,
    net::{IpAddr, SocketAddr},
    str::FromStr,
};

use bitcode::{Decode, Encode};
use derive_more::{AsRef, Constructor, Display, Error, From, IsVariant};

use crate::server::{NETWORK_PORT, PeerId};

pub const MAX_SHARE_NAME_LENGTH: usize = 256;

#[derive(Encode, Decode, Constructor, Clone, Debug)]
pub struct ShareLocator {
    pub authority: Authority,
    pub share_name: ShareName,
    pub fragment: Option<String>,
}

impl fmt::Display for ShareLocator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "rdir://{}/{}", self.authority, self.share_name)?;

        if let Some(fragment) = &self.fragment
            && !fragment.is_empty()
        {
            write!(f, "#{fragment}")?;
        }

        Ok(())
    }
}

impl FromStr for ShareLocator {
    type Err = ShareLocatorParseError;

    fn from_str(mut s: &str) -> Result<Self, Self::Err> {
        if let Some((schema, rest)) = s.split_once("://") {
            if schema != "rdir" {
                return Err(ShareLocatorParseError::InvalidSchema);
            }
            s = rest;
        }

        let (raw_authority, s) = s
            .split_once('/')
            .ok_or(ShareLocatorParseError::NoShareName)?;
        let authority = Authority::from_str(raw_authority)?;

        let (share_name, fragment) = match s.split_once('#') {
            Some((share_name, fragment)) => (share_name, Some(fragment.to_string())),
            None => (s, None),
        };

        Ok(Self {
            authority,
            share_name: share_name.parse()?,
            fragment,
        })
    }
}

#[derive(Encode, Decode, Clone, Debug, Display, Error, From, PartialEq, Eq, IsVariant)]
pub enum ShareLocatorParseError {
    #[display("URL schema is not \"rdir\"")]
    InvalidSchema,
    AuthorityParse(AuthorityParseError),
    #[display("No share name given in URL path")]
    NoShareName,
    #[display("Share name is too long")]
    ShareNameTooLong(ValueTooLong),
}

impl From<(SocketAddr, ShareName)> for ShareLocator {
    fn from(value: (SocketAddr, ShareName)) -> Self {
        let (addr, name) = value;
        Self::new(addr.into(), name, None)
    }
}

#[derive(Encode, Decode, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Authority {
    pub domain: Domain,
    pub port: Option<u16>,
}

impl fmt::Display for Authority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.domain)?;

        if let Some(port) = &self.port {
            write!(f, ":{port}")?;
        }

        Ok(())
    }
}

impl FromStr for Authority {
    type Err = AuthorityParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (domain, port) = match s.split_once(':') {
            Some((domain, port)) => {
                let port = port
                    .parse::<u16>()
                    .map_err(|_| AuthorityParseError::InvalidPort)?;
                (domain, (port != NETWORK_PORT).then_some(port))
            }
            None => (s, None),
        };
        Ok(Self {
            domain: domain.parse()?,
            port,
        })
    }
}

#[derive(Encode, Decode, Clone, Debug, Display, Error, From, PartialEq, Eq, IsVariant)]
pub enum AuthorityParseError {
    #[display("Failed to parse the port number")]
    InvalidPort,
    DomainParse(ValueTooLong),
}

impl From<SocketAddr> for Authority {
    fn from(value: SocketAddr) -> Self {
        Self {
            domain: value.ip().into(),
            port: Some(value.port()),
        }
    }
}

#[derive(Encode, Decode, Clone, Debug, Display, IsVariant, PartialEq, Eq, PartialOrd, Ord)]
#[display("{_0}")]
pub enum Domain {
    #[display("localhost")]
    Local,
    PeerId(PeerId),
    Remote(String),
}

impl FromStr for Domain {
    type Err = ValueTooLong;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        const MAX_DNS_AUTHORITY_LENGTH: usize = 256;
        if s.len() > MAX_DNS_AUTHORITY_LENGTH {
            return Err(ValueTooLong(MAX_DNS_AUTHORITY_LENGTH));
        }

        Ok(Self::Remote(s.to_string()))
    }
}

impl From<SocketAddr> for Domain {
    fn from(value: SocketAddr) -> Self {
        Self::Remote(value.to_string())
    }
}

impl From<IpAddr> for Domain {
    fn from(value: IpAddr) -> Self {
        Self::Remote(value.to_string())
    }
}

#[derive(Encode, Decode, AsRef, Clone, Debug, Display, PartialEq, Eq, PartialOrd, Ord)]
pub struct ShareName(String);

impl FromStr for ShareName {
    type Err = ValueTooLong;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() > MAX_SHARE_NAME_LENGTH {
            return Err(ValueTooLong(MAX_SHARE_NAME_LENGTH));
        }

        Ok(Self(s.to_string()))
    }
}

#[derive(Encode, Decode, Clone, Debug, Display, Error, PartialEq, Eq)]
#[display("Name of a share cannot exceed {_0} characters")]
pub struct ValueTooLong(#[error(ignore)] usize);
