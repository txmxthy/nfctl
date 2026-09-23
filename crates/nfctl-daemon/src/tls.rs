use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use rustls_platform_verifier::BuilderVerifierExt;

/// Accepts any certificate. The daemon mints a self-signed certificate on every
/// start, so the authenticated Kubernetes port-forward is the trust boundary.
#[derive(Debug)]
struct NoVerify(Arc<CryptoProvider>);

impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// TLS config that accepts any certificate. The port-forward transport uses
/// it because the API server has already authenticated the far end; a direct
/// URL uses it only when the caller asks with `--daemon-insecure`.
pub(crate) fn unverified_client_config() -> Arc<rustls::ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions();
    let mut cfg = match builder {
        Ok(b) => b
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerify(provider)))
            .with_no_client_auth(),
        // ring always supports TLS 1.2 and 1.3; falling back to defaults keeps this infallible.
        Err(_) => rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
            .with_protocol_versions(rustls::DEFAULT_VERSIONS)
            .unwrap_or_else(|_| unreachable!("ring supports the default protocol versions"))
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerify(provider)))
            .with_no_client_auth(),
    };
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(cfg)
}

/// TLS config for direct HTTPS, using the operating system's trust decisions.
pub(crate) fn direct_client_config() -> Result<Arc<rustls::ClientConfig>, rustls::Error> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut cfg = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_platform_verifier()?
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(cfg))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use rustls::pki_types::PrivateKeyDer;
    use rustls::{ClientConnection, ServerConfig, ServerConnection};

    use super::*;

    /// Runs a handshake in memory against a server that presents a fresh
    /// self-signed certificate, the way a Numaflow daemon does.
    fn handshake_with_self_signed(cfg: Arc<rustls::ClientConfig>) -> Result<(), rustls::Error> {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
        let key = PrivateKeyDer::Pkcs8(cert.signing_key.serialize_der().into());
        let server_cfg =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(vec![cert.cert.der().clone()], key)
                .unwrap();
        let mut server = ServerConnection::new(Arc::new(server_cfg)).unwrap();
        let mut client =
            ClientConnection::new(cfg, ServerName::try_from("localhost").unwrap()).unwrap();

        while client.is_handshaking() || server.is_handshaking() {
            let mut moved = false;
            if client.wants_write() {
                let mut wire = Vec::new();
                client.write_tls(&mut wire).unwrap();
                let mut wire = wire.as_slice();
                while !wire.is_empty() {
                    server.read_tls(&mut wire).unwrap();
                }
                server.process_new_packets()?;
                moved = true;
            }
            if server.wants_write() {
                let mut wire = Vec::new();
                server.write_tls(&mut wire).unwrap();
                let mut wire = wire.as_slice();
                while !wire.is_empty() {
                    client.read_tls(&mut wire).unwrap();
                }
                client.process_new_packets()?;
                moved = true;
            }
            assert!(moved, "the handshake stalled");
        }
        Ok(())
    }

    #[test]
    fn a_verified_direct_connection_rejects_a_self_signed_daemon() {
        let err = handshake_with_self_signed(direct_client_config().unwrap()).unwrap_err();
        assert!(
            matches!(err, rustls::Error::InvalidCertificate(_)),
            "{err:?}"
        );
    }

    #[test]
    fn the_unverified_config_accepts_a_self_signed_daemon() {
        handshake_with_self_signed(unverified_client_config()).unwrap();
    }
}
