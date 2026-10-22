use std::{
    fs::{self, File},
    io::{self, Write},
    path::Path,
    str::FromStr,
    sync::Arc,
};

use derive_more::{Display, Error, From, IsVariant};
use quinn::rustls::{
    self, ServerConfig,
    crypto::{CryptoProvider, WebPkiSupportedAlgorithms},
    pki_types::{CertificateDer, PrivatePkcs8KeyDer},
    server::danger::ClientCertVerifier,
};
use x509_parser::error::X509Error;

use crate::server::PeerId;

pub struct Identity {
    peer_id: PeerId,
    cert: CertificateDer<'static>,
    key: PrivatePkcs8KeyDer<'static>,
}

impl Identity {
    pub fn new() -> Result<Self, IdentityError> {
        let signing_key = rcgen::KeyPair::generate()?;
        let mut cert_params = rcgen::CertificateParams::default();
        // By default it's filled with a CN
        cert_params.distinguished_name = Default::default();
        cert_params
            .key_usages
            .push(rcgen::KeyUsagePurpose::DigitalSignature);
        cert_params
            .extended_key_usages
            .push(rcgen::ExtendedKeyUsagePurpose::ServerAuth);
        cert_params
            .extended_key_usages
            .push(rcgen::ExtendedKeyUsagePurpose::ClientAuth);
        let cert = cert_params.self_signed(&signing_key)?;

        let cert = cert.der().to_owned();
        Ok(Identity {
            peer_id: peer_id_from_certder(&cert)?,
            cert,
            key: PrivatePkcs8KeyDer::<'static>::from(signing_key.serialize_der()),
        })
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

    pub fn into_server_config(self) -> Result<(ServerConfig, PeerId), IdentityError> {
        let Self { peer_id, cert, key } = self;
        let server_config = ServerConfig::builder()
            .with_client_cert_verifier(Arc::new(ClientVerifier::new()?))
            .with_single_cert(vec![cert], key.into())?;
        Ok((server_config, peer_id))
    }
}

pub fn peer_id_from_certder(cert: &CertificateDer<'_>) -> Result<PeerId, IdentityError> {
    let parsed_cert = x509_parser::parse_x509_certificate(cert)?.1;
    Ok(PeerId::from_raw_pubkey(parsed_cert.public_key().raw))
}

#[derive(Debug, Display, Error, From, IsVariant)]
pub enum IdentityError {
    Io(io::Error),
    Rcgen(rcgen::Error),
    Rustls(rustls::Error),
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
    NoCryptoProvider,
}

#[derive(Debug)]
struct ClientVerifier {
    supported_algs: WebPkiSupportedAlgorithms,
}

impl ClientVerifier {
    /// Returns None if no default crypto provider is found
    fn new() -> Result<Self, IdentityError> {
        let crypto = CryptoProvider::get_default().ok_or(IdentityError::NoCryptoProvider)?;
        Ok(Self {
            supported_algs: crypto.signature_verification_algorithms,
        })
    }
}

impl ClientCertVerifier for ClientVerifier {
    fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::server::danger::ClientCertVerified, rustls::Error> {
        if !intermediates.is_empty() {
            return Err(rustls::Error::InvalidMessage(
                rustls::InvalidMessage::MessageTooLarge,
            ));
        }

        let map_x509_error = |e| {
            rustls::Error::InvalidCertificate(match e {
                x509_parser::nom::Err::Error(e) | x509_parser::nom::Err::Failure(e) => {
                    rustls::CertificateError::Other(rustls::OtherError(Arc::new(e)))
                }
                e => rustls::CertificateError::Other(rustls::OtherError(Arc::new(e))),
            })
        };

        let cert = x509_parser::parse_x509_certificate(end_entity)
            .map_err(map_x509_error)?
            .1;

        if !(cert.subject().iter_rdn().next().is_none()
            && cert.issuer().iter_rdn().next().is_none())
        {
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer,
            ));
        }

        let now = x509_parser::time::ASN1Time::from_timestamp(now.as_secs() as i64).unwrap();
        if now < cert.validity.not_before {
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::NotValidYet,
            ));
        }
        if cert.validity.not_after < now {
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::Expired,
            ));
        }

        if cert
            .key_usage()
            .map_err(|e| map_x509_error(x509_parser::asn1_rs::Err::<X509Error>::Error(e)))?
            .is_none_or(|key_usage| !key_usage.value.digital_signature())
            || cert
                .extended_key_usage()
                .map_err(|e| map_x509_error(x509_parser::asn1_rs::Err::<X509Error>::Error(e)))?
                .is_none_or(|key_usage| !key_usage.value.client_auth)
        {
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::InvalidPurpose,
            ));
        }
        todo!()
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.supported_algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.supported_algs)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.supported_algs.supported_schemes()
    }
}
