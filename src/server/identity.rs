use std::{
    fs::{self, File},
    io::{self, Write},
    path::Path,
    str::FromStr,
};

use derive_more::{Display, Error, From, IsVariant};
use quinn::rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use tracing::debug;

use crate::server::PeerId;

pub struct Identity {
    peer_id: PeerId,
    cert: CertificateDer<'static>,
    key: PrivatePkcs8KeyDer<'static>,
}

impl Identity {
    pub fn new() -> Result<Self, IdentityError> {
        let rcgen::CertifiedKey { cert, signing_key } = rcgen::generate_simple_self_signed([])?;
        let cert = cert.der().to_owned();
        Ok(Identity {
            peer_id: peer_id_from_certder(&cert)?,
            cert,
            key: PrivatePkcs8KeyDer::<'static>::from(signing_key.serialize_der()),
        })
    }

    pub fn peer_id(&self) -> &PeerId {
        &self.peer_id
    }

    pub fn try_load(dir: impl AsRef<Path>) -> Result<Self, IdentityError> {
        let (specified_peer_id, path) = dir
            .as_ref()
            .read_dir()?
            .find_map(|r| {
                r.map(|e| {
                    let filename_raw = e.file_name();
                    let filename_parsed = filename_raw.to_string_lossy();
                    let (filename, extension) = filename_parsed.rsplit_once('.')?;
                    if extension == "pem"
                        && let Ok(peer_id) = PeerId::from_str(filename)
                    {
                        Some((peer_id, e.path()))
                    } else {
                        None
                    }
                })
                .transpose()
            })
            .ok_or(IdentityError::NoIdentityFound)??;

        let pem = fs::read(path)?;
        let mut cert_reader = io::BufReader::new(pem.as_slice());
        let mut iter = rustls_pemfile::read_all(&mut cert_reader);

        let rustls_pemfile::Item::X509Certificate(cert) =
            iter.next().ok_or(IdentityError::InvalidIdentityFile)??
        else {
            return Err(IdentityError::InvalidIdentityFile);
        };

        let rustls_pemfile::Item::Pkcs8Key(key) =
            iter.next().ok_or(IdentityError::InvalidIdentityFile)??
        else {
            return Err(IdentityError::InvalidIdentityFile);
        };

        let peer_id = peer_id_from_certder(&cert)?;

        if peer_id != specified_peer_id {
            return Err(IdentityError::PeerIdDoesntMatch);
        }

        Ok(Self { peer_id, cert, key })
    }

    pub fn load_or_new(dir: impl AsRef<Path>) -> Result<Identity, IdentityError> {
        match Self::try_load(dir.as_ref()) {
            Ok(identity) => Ok(identity),
            Err(IdentityError::NoIdentityFound) => {
                let ident = Self::new()?;
                ident.write(dir)?;
                Ok(ident)
            }
            Err(err) => Err(err),
        }
    }

    /// Write this identity into the given directory
    pub fn write(&self, dir: impl AsRef<Path>) -> io::Result<()> {
        let mut path = dir.as_ref().to_owned();
        path.push(format!("{}.pem", self.peer_id));
        let mut file = File::create_new(path)?;

        let cert_pem = pem::Pem::new("CERTIFICATE", self.cert.as_ref()).to_string();
        let key_pem = pem::Pem::new("PRIVATE KEY", self.key.secret_pkcs8_der()).to_string();
        write!(file, "{}{}", cert_pem, key_pem)?;
        Ok(())
    }
}

fn peer_id_from_certder(cert: &CertificateDer<'_>) -> Result<PeerId, IdentityError> {
    let parsed_cert = x509_parser::parse_x509_certificate(&cert)?.1;
    Ok(PeerId::from_raw_pubkey(parsed_cert.public_key().raw))
}

#[derive(Debug, Display, Error, From, IsVariant)]
pub enum IdentityError {
    Io(io::Error),
    Rcgen(rcgen::Error),
    #[display("Failed to parse the private key as DER: {_0}")]
    Der(
        #[error(ignore)]
        #[from(ignore)]
        &'static str,
    ),
    #[display("No identity file found")]
    NoIdentityFound,
    InvalidIdentityFile,
    #[display("The peer id derived from the identity file, doesn't match it's name")]
    PeerIdDoesntMatch,
    InvalidCert(x509_parser::asn1_rs::Err<x509_parser::error::X509Error>),
}
