//! TLS identity and trust primitives for agent-node mode (roadmap
//! phase 67, Stage 1).
//!
//! Canonical Roder-to-Roder security is `wss://` with mTLS: the server
//! certificate identifies the node and the client certificate identifies
//! the controller. Trust is fingerprint-pinned in both directions — the
//! node pins enrolled controller certificate fingerprints, and controllers
//! pin the node's certificate fingerprint from pairing output. Private
//! keys never leave the machine that generated them.

use std::sync::Arc;

use anyhow::Context;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};
use sha2::{Digest, Sha256};

/// A generated TLS identity (certificate + private key, PEM-encoded).
#[derive(Clone)]
pub struct TlsIdentity {
    pub cert_pem: String,
    pub key_pem: String,
    /// Lowercase hex SHA-256 of the DER certificate.
    pub fingerprint: String,
}

impl std::fmt::Debug for TlsIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsIdentity")
            .field("fingerprint", &self.fingerprint)
            .field("key_pem", &"<redacted>")
            .finish()
    }
}

/// Generates a self-signed identity for a node or controller. Node
/// identities carry loopback + hostname SANs so local controllers can
/// connect by IP or name.
pub fn generate_identity(common_name: &str) -> anyhow::Result<TlsIdentity> {
    let mut params = rcgen::CertificateParams::new(vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        common_name.to_string(),
    ])?;
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, common_name);
    let key_pair = rcgen::KeyPair::generate()?;
    let certificate = params.self_signed(&key_pair)?;
    let cert_pem = certificate.pem();
    let key_pem = key_pair.serialize_pem();
    let fingerprint = fingerprint_from_pem(&cert_pem)?;
    Ok(TlsIdentity {
        cert_pem,
        key_pem,
        fingerprint,
    })
}

/// Lowercase hex SHA-256 fingerprint of the first certificate in `pem`.
pub fn fingerprint_from_pem(pem: &str) -> anyhow::Result<String> {
    let der = first_cert_der(pem)?;
    Ok(fingerprint_der(&der))
}

pub fn fingerprint_der(der: &CertificateDer<'_>) -> String {
    let digest = Sha256::digest(der.as_ref());
    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

pub(crate) fn first_cert_der(pem: &str) -> anyhow::Result<CertificateDer<'static>> {
    let mut reader = std::io::BufReader::new(pem.as_bytes());
    rustls_pemfile::certs(&mut reader)
        .next()
        .context("PEM contains no certificate")?
        .context("invalid certificate PEM")
}

fn private_key_der(pem: &str) -> anyhow::Result<PrivateKeyDer<'static>> {
    let mut reader = std::io::BufReader::new(pem.as_bytes());
    rustls_pemfile::private_key(&mut reader)?.context("PEM contains no private key")
}

/// Installs the ring crypto provider exactly once. Other dependencies can
/// enable additional rustls backends, which makes the automatic provider
/// selection ambiguous; agent-node TLS pins ring explicitly.
fn ensure_crypto_provider() {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Server-side TLS config: presents the node identity and *requests* a
/// client certificate. Client certificates are accepted at the TLS layer
/// and authorized (fingerprint pinning) after the handshake so that
/// unenrolled controllers can still reach the token-gated enrollment
/// path over an encrypted channel.
pub fn server_tls_config(identity: &TlsIdentity) -> anyhow::Result<rustls::ServerConfig> {
    ensure_crypto_provider();
    let cert = first_cert_der(&identity.cert_pem)?;
    let key = private_key_der(&identity.key_pem)?;
    let config = rustls::ServerConfig::builder()
        .with_client_cert_verifier(Arc::new(CaptureClientCerts))
        .with_single_cert(vec![cert], key)?;
    Ok(config)
}

/// Client-side TLS config pinning the node's certificate fingerprint and
/// optionally presenting a controller identity for mTLS.
pub fn client_tls_config(
    server_fingerprint: &str,
    controller_identity: Option<&TlsIdentity>,
) -> anyhow::Result<rustls::ClientConfig> {
    ensure_crypto_provider();
    let builder = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedServerCert {
            fingerprint: server_fingerprint.to_ascii_lowercase(),
        }));
    let config = match controller_identity {
        Some(identity) => builder.with_client_auth_cert(
            vec![first_cert_der(&identity.cert_pem)?],
            private_key_der(&identity.key_pem)?,
        )?,
        None => builder.with_no_client_auth(),
    };
    Ok(config)
}

/// TLS-layer client-cert handler: accepts any presented certificate (and
/// connections without one) so authorization can happen by fingerprint
/// after the handshake. Control-plane requests are rejected before
/// dispatch unless the connection's certificate is enrolled.
#[derive(Debug)]
struct CaptureClientCerts;

impl ClientCertVerifier for CaptureClientCerts {
    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        false
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Server-certificate verifier that pins one SHA-256 fingerprint.
#[derive(Debug)]
struct PinnedServerCert {
    fingerprint: String,
}

impl ServerCertVerifier for PinnedServerCert {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if fingerprint_der(end_entity) == self.fingerprint {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(
                "server certificate fingerprint does not match the pinned node identity"
                    .to_string(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
