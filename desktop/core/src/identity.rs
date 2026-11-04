//! The PC's identity: a long-lived self-signed certificate.
//!
//! The phone never trusts a certificate authority. At pairing it learns the SHA-256
//! fingerprint of this certificate from the QR code and pins it. The host name and
//! IP inside the certificate are irrelevant to a pinning client, so a changing LAN
//! address never breaks an existing pairing.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rcgen::{CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, KeyPair};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::crypto::{ring, CryptoProvider};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::error::{Error, Result};
use crate::util::open_private;

const CERT_FILE: &str = "identity-cert.pem";
const KEY_FILE: &str = "identity-key.pem";

pub struct Identity {
    pub cert: CertificateDer<'static>,
    pub key: PrivateKeyDer<'static>,
    /// SHA-256 of the DER certificate, lowercase hex.
    pub fingerprint: String,
}

pub fn fingerprint_of(der: &[u8]) -> String {
    hex::encode(Sha256::digest(der))
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(ring::default_provider())
}

/// Load the identity from `dir`, creating it on first run.
pub fn load_or_create(dir: &Path, lan_host: &str) -> Result<Identity> {
    let cert_path = dir.join(CERT_FILE);
    let key_path = dir.join(KEY_FILE);
    if !(cert_path.exists() && key_path.exists()) {
        create(dir, lan_host, &cert_path, &key_path)?;
    }
    let cert_pem = std::fs::read(&cert_path)?;
    let key_pem = std::fs::read(&key_path)?;
    let cert = rustls_pemfile::certs(&mut cert_pem.as_slice())
        .next()
        .ok_or_else(|| Error::Tls("no certificate in identity file".into()))??;
    let key = rustls_pemfile::private_key(&mut key_pem.as_slice())?
        .ok_or_else(|| Error::Tls("no private key in identity file".into()))?;
    let fingerprint = fingerprint_of(cert.as_ref());
    Ok(Identity { cert, key, fingerprint })
}

fn create(dir: &Path, lan_host: &str, cert_path: &Path, key_path: &Path) -> Result<()> {
    use std::io::Write;
    let mut sans = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    if lan_host != "127.0.0.1" {
        sans.push(lan_host.to_string());
    }
    let tls = |e: rcgen::Error| Error::Tls(e.to_string());
    let mut params = CertificateParams::new(sans).map_err(tls)?;
    let mut name = DistinguishedName::new();
    name.push(DnType::CommonName, "bridge-pc");
    params.distinguished_name = name;
    params.not_before = rcgen::date_time_ymd(2020, 1, 1);
    params.not_after = rcgen::date_time_ymd(2046, 1, 1);
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let key = KeyPair::generate().map_err(tls)?; // ECDSA P-256
    let cert = params.self_signed(&key).map_err(tls)?;

    std::fs::create_dir_all(dir)?;
    // Key first, with owner-only permissions from the moment it exists.
    open_private(key_path)?.write_all(key.serialize_pem().as_bytes())?;
    std::fs::write(cert_path, cert.pem())?;
    Ok(())
}

/// TLS settings for the phone-facing server: TLS 1.2 or newer, no client certificates.
pub fn server_config(id: &Identity) -> Result<Arc<rustls::ServerConfig>> {
    let cfg = rustls::ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Tls(e.to_string()))?
        .with_no_client_auth()
        .with_single_cert(vec![id.cert.clone()], id.key.clone_key())
        .map_err(|e| Error::Tls(e.to_string()))?;
    Ok(Arc::new(cfg))
}

/// Client-side TLS settings that accept exactly one certificate: the one whose
/// SHA-256 fingerprint is `fingerprint_hex`. This is what the Android app will do
/// on every connection. Handshake signatures are still fully verified.
pub fn pinned_client_config(fingerprint_hex: &str) -> Result<rustls::ClientConfig> {
    let want: [u8; 32] = hex::decode(fingerprint_hex)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| Error::Invalid("fingerprint must be 64 hex characters".into()))?;
    let provider = provider();
    let verifier = PinVerifier { want, provider: provider.clone() };
    let cfg = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Tls(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    Ok(cfg)
}

#[derive(Debug)]
struct PinVerifier {
    want: [u8; 32],
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let got = Sha256::digest(end_entity.as_ref());
        if bool::from(got.as_slice().ct_eq(&self.want)) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("certificate does not match the pinned fingerprint".into()))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}


// ---------------------------------------------------------------------------------------------
// PC <-> PC: mutual TLS. Each PC presents its own self-signed certificate; identity is the
// certificate's SHA-256 fingerprint, checked against the list of paired PCs by the application.
// ---------------------------------------------------------------------------------------------

/// Server side of the PC port: a client certificate is required, any certificate is accepted at the
/// TLS level (the handshake signature is still verified, so the peer proves it holds the key), and the
/// application then decides by fingerprint whether this PC is paired.
#[derive(Debug)]
struct AnyClientCert {
    provider: Arc<CryptoProvider>,
}

impl ClientCertVerifier for AnyClientCert {
    fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> std::result::Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// TLS settings for the PC-facing port.
pub fn peer_server_config(id: &Identity) -> Result<Arc<rustls::ServerConfig>> {
    let cfg = rustls::ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Tls(e.to_string()))?
        .with_client_cert_verifier(Arc::new(AnyClientCert { provider: provider() }))
        .with_single_cert(vec![id.cert.clone()], id.key.clone_key())
        .map_err(|e| Error::Tls(e.to_string()))?;
    Ok(Arc::new(cfg))
}

/// Remembers the fingerprint of the certificate the server presented. With a pin set, any other
/// certificate is refused; without one (first contact while pairing) the certificate is only recorded.
#[derive(Debug)]
struct PeerVerifier {
    pin: Option<[u8; 32]>,
    observed: Arc<Mutex<Option<String>>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PeerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let got = Sha256::digest(end_entity.as_ref());
        *self.observed.lock().unwrap_or_else(|e| e.into_inner()) = Some(hex::encode(got));
        match &self.pin {
            Some(want) if !bool::from(got.as_slice().ct_eq(want)) => {
                Err(rustls::Error::General("certificate does not match the paired PC".into()))
            }
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// Client settings for talking to another PC: presents `id` as the client certificate and checks the
/// server against `pin` (a fingerprint). `observed` receives the fingerprint the server showed.
pub(crate) fn peer_client_config(
    id: &Identity,
    pin: Option<&str>,
    observed: Arc<Mutex<Option<String>>>,
) -> Result<rustls::ClientConfig> {
    let pin = match pin {
        None => None,
        Some(hex_fp) => Some(
            hex::decode(hex_fp)
                .ok()
                .and_then(|v| <[u8; 32]>::try_from(v).ok())
                .ok_or_else(|| Error::Invalid("fingerprint must be 64 hex characters".into()))?,
        ),
    };
    let provider = provider();
    rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Tls(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PeerVerifier { pin, observed, provider }))
        .with_client_auth_cert(vec![id.cert.clone()], id.key.clone_key())
        .map_err(|e| Error::Tls(e.to_string()))
}

/// Client TLS settings that behave like another PC, using the identity stored in `data_dir`
/// (created if missing). Meant for tests and tools that need to talk to the PC port.
pub fn peer_client_config_from_dir(data_dir: &Path, pin: Option<&str>) -> Result<rustls::ClientConfig> {
    let id = load_or_create(data_dir, "127.0.0.1")?;
    peer_client_config(&id, pin, Arc::new(Mutex::new(None)))
}

/// Fingerprint of the first certificate a client presented on this connection, if any.
pub(crate) fn peer_fingerprint(conn: &rustls::ServerConnection) -> Option<String> {
    conn.peer_certificates().and_then(|c| c.first()).map(|c| fingerprint_of(c.as_ref()))
}
