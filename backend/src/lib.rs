pub mod audit;
pub mod config;
pub mod domain;
pub mod error;
pub mod handlers;
pub mod metrics;
pub mod middleware;
pub mod router;
pub mod services;
pub mod state;
pub mod ws;

pub use config::Config;
pub use state::AppState;

/// Both reqwest and SMTP use rustls. Their dependency features can enable more
/// than one crypto backend, so choose one before either builds a TLS client.
pub fn install_tls_crypto_provider() {
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
}
