#[test]
fn tls_client_can_be_built_after_provider_initialization() {
    cs_mail_api::install_tls_crypto_provider();

    assert!(tokio_rustls::rustls::crypto::CryptoProvider::get_default().is_some());
    let _client = tokio_rustls::rustls::ClientConfig::builder()
        .with_root_certificates(tokio_rustls::rustls::RootCertStore::empty())
        .with_no_client_auth();
}
