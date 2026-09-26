//! Private attachment object storage.
//!
//! `local` keeps attachment bytes in the persistent API volume. `r2` uses
//! Cloudflare R2's S3-compatible API while retaining the local volume as a
//! private upload/import spool. R2 credentials never reach the browser.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use hmac::{Hmac, Mac};
use reqwest::{Client, Method, StatusCode, Url};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug)]
pub struct R2Config {
    pub account_id: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub endpoint: String,
    pub region: String,
    pub prefix: String,
    pub request_timeout_secs: u64,
    pub max_concurrent_transfers: usize,
}

#[derive(Clone)]
pub struct ObjectStore {
    local_dir: PathBuf,
    backend: String,
    r2: Option<R2Store>,
}

#[derive(Clone)]
struct R2Store {
    client: Client,
    config: R2Config,
    transfer_limit: Arc<Semaphore>,
}

impl ObjectStore {
    pub fn new(local_dir: PathBuf, backend: &str, r2: Option<R2Config>) -> Result<Self, String> {
        let backend = backend.trim().to_ascii_lowercase();
        if !matches!(backend.as_str(), "local" | "r2") {
            return Err("CS_MAIL_OBJECT_STORAGE_BACKEND must be local or r2".to_string());
        }
        let r2_store = match r2 {
            Some(config) => Some(R2Store::new(config)?),
            None => None,
        };
        if backend == "r2" && r2_store.is_none() {
            return Err("Cloudflare R2 is selected but its credentials are incomplete".to_string());
        }
        Ok(Self { local_dir, backend, r2: r2_store })
    }

    pub fn active_backend(&self) -> &'static str {
        if self.backend == "r2" { "r2" } else { "local" }
    }

    pub fn local_path(&self, key: &str) -> Result<PathBuf, String> {
        let rel = checked_relative_key(key)?;
        Ok(self.local_dir.join(rel))
    }

    pub async fn healthcheck(&self) -> Result<(), String> {
        if self.backend == "r2" {
            self.r2.as_ref().ok_or_else(|| "R2 is not configured".to_string())?.healthcheck().await?;
        }
        Ok(())
    }

    pub async fn commit_local_file(&self, key: &str, path: &Path, content_type: &str) -> Result<&'static str, String> {
        if self.backend == "r2" {
            self.r2.as_ref().ok_or_else(|| "R2 is not configured".to_string())?
                .put_file(key, path, content_type).await?;
            if let Err(error) = tokio::fs::remove_file(path).await {
                if error.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(%error, path=%path.display(), "R2 upload committed but local spool cleanup failed");
                }
            }
            Ok("r2")
        } else {
            Ok("local")
        }
    }

    pub async fn put_bytes(&self, key: &str, bytes: &[u8], content_type: &str) -> Result<&'static str, String> {
        if self.backend == "r2" {
            self.r2.as_ref().ok_or_else(|| "R2 is not configured".to_string())?
                .put(key, bytes.to_vec(), content_type).await?;
            Ok("r2")
        } else {
            let path = self.local_path(key)?;
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(|e| format!("Unable to prepare object directory: {e}"))?;
            }
            tokio::fs::write(path, bytes).await.map_err(|e| format!("Unable to write object: {e}"))?;
            Ok("local")
        }
    }

    pub async fn get_bytes(&self, key: &str, stored_backend: &str) -> Result<Option<Vec<u8>>, String> {
        match stored_backend {
            "r2" => self.r2.as_ref().ok_or_else(|| "Attachment is stored in R2 but R2 is not configured".to_string())?.get(key).await,
            "local" => {
                let path = self.local_path(key)?;
                match tokio::fs::read(path).await {
                    Ok(bytes) => Ok(Some(bytes)),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(error) => Err(format!("Unable to read local object: {error}")),
                }
            }
            other => Err(format!("Unknown attachment storage backend: {other}")),
        }
    }

    pub async fn delete(&self, key: &str, stored_backend: &str) -> Result<(), String> {
        match stored_backend {
            "r2" => self.r2.as_ref().ok_or_else(|| "Attachment is stored in R2 but R2 is not configured".to_string())?.delete(key).await,
            "local" => self.delete_local(key).await,
            other => Err(format!("Unknown attachment storage backend: {other}")),
        }
    }

    pub async fn delete_local(&self, key: &str) -> Result<(), String> {
        let path = self.local_path(key)?;
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("Unable to delete local object: {error}")),
        }
    }

    pub async fn delete_local_mailbox_dirs(&self, mailbox_id: uuid::Uuid) -> Result<(), String> {
        for rel in [mailbox_id.to_string(), format!("imports/{mailbox_id}")] {
            let path = self.local_path(&rel)?;
            match tokio::fs::remove_dir_all(path).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("Unable to clean mailbox spool: {error}")),
            }
        }
        Ok(())
    }
}

fn checked_relative_key(key: &str) -> Result<PathBuf, String> {
    let rel = PathBuf::from(key);
    if key.trim().is_empty()
        || rel.is_absolute()
        || rel.components().any(|component| matches!(component, Component::ParentDir | Component::RootDir | Component::Prefix(_)))
    {
        return Err("Invalid object storage key".to_string());
    }
    Ok(rel)
}

impl R2Store {
    fn new(mut config: R2Config) -> Result<Self, String> {
        config.account_id = config.account_id.trim().to_string();
        config.bucket = config.bucket.trim().to_string();
        config.access_key_id = config.access_key_id.trim().to_string();
        config.secret_access_key = config.secret_access_key.trim().to_string();
        config.region = if config.region.trim().is_empty() { "auto".to_string() } else { config.region.trim().to_string() };
        config.prefix = config.prefix.trim_matches('/').to_string();
        config.endpoint = config.endpoint.trim().trim_end_matches('/').to_string();
        if config.endpoint.is_empty() {
            config.endpoint = format!("https://{}.r2.cloudflarestorage.com", config.account_id);
        }
        if config.account_id.is_empty() || config.bucket.is_empty() || config.access_key_id.is_empty() || config.secret_access_key.is_empty() {
            return Err("R2 account id, bucket, access key id and secret access key are required".to_string());
        }
        let parsed = Url::parse(&config.endpoint).map_err(|e| format!("Invalid R2 endpoint: {e}"))?;
        if parsed.scheme() != "https" || parsed.host_str().is_none() {
            return Err("CS_MAIL_R2_ENDPOINT must be an HTTPS URL".to_string());
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(config.request_timeout_secs.max(5)))
            .build()
            .map_err(|e| format!("Unable to build R2 client: {e}"))?;
        let transfer_limit = Arc::new(Semaphore::new(config.max_concurrent_transfers.clamp(1, 16)));
        Ok(Self { client, config, transfer_limit })
    }

    fn object_key(&self, key: &str) -> Result<String, String> {
        checked_relative_key(key)?;
        if self.config.prefix.is_empty() {
            Ok(key.trim_start_matches('/').to_string())
        } else {
            Ok(format!("{}/{}", self.config.prefix, key.trim_start_matches('/')))
        }
    }

    fn url_and_uri(&self, key: Option<&str>) -> Result<(String, String, String), String> {
        let endpoint = Url::parse(&self.config.endpoint).map_err(|e| format!("Invalid R2 endpoint: {e}"))?;
        let host = match endpoint.port() {
            Some(port) => format!("{}:{port}", endpoint.host_str().unwrap_or_default()),
            None => endpoint.host_str().unwrap_or_default().to_string(),
        };
        let suffix = match key {
            Some(key) => format!("/{}/{}", self.config.bucket, aws_uri_encode(&self.object_key(key)?)),
            None => format!("/{}", self.config.bucket),
        };
        Ok((format!("{}{}", self.config.endpoint, suffix), suffix, host))
    }

    fn signed_headers(&self, method: &Method, canonical_uri: &str, host: &str, payload_hash: &str) -> Result<(String, String, String), String> {
        let now = Utc::now();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let date = now.format("%Y%m%d").to_string();
        let canonical_headers = format!("host:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amz_date}\n");
        let signed_headers = "host;x-amz-content-sha256;x-amz-date";
        let canonical_request = format!("{}\n{}\n\n{}\n{}\n{}", method.as_str(), canonical_uri, canonical_headers, signed_headers, payload_hash);
        let canonical_hash = hex_sha256(canonical_request.as_bytes());
        let scope = format!("{date}/{}/s3/aws4_request", self.config.region);
        let string_to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{canonical_hash}");
        let k_date = hmac_bytes(format!("AWS4{}", self.config.secret_access_key).as_bytes(), date.as_bytes())?;
        let k_region = hmac_bytes(&k_date, self.config.region.as_bytes())?;
        let k_service = hmac_bytes(&k_region, b"s3")?;
        let k_signing = hmac_bytes(&k_service, b"aws4_request")?;
        let signature = hex_bytes(&hmac_bytes(&k_signing, string_to_sign.as_bytes())?);
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            self.config.access_key_id, scope, signed_headers, signature
        );
        Ok((amz_date, authorization, payload_hash.to_string()))
    }

    async fn request(&self, method: Method, key: Option<&str>, body: Option<Vec<u8>>, content_type: Option<&str>) -> Result<reqwest::Response, String> {
        let (url, canonical_uri, host) = self.url_and_uri(key)?;
        let payload = body.unwrap_or_default();
        let payload_hash = hex_sha256(&payload);
        let (amz_date, authorization, payload_hash) = self.signed_headers(&method, &canonical_uri, &host, &payload_hash)?;
        let mut request = self.client.request(method, url)
            .header("x-amz-date", amz_date)
            .header("x-amz-content-sha256", payload_hash)
            .header("authorization", authorization);
        if let Some(content_type) = content_type {
            request = request.header("content-type", content_type);
        }
        if !payload.is_empty() {
            request = request.body(payload);
        }
        request.send().await.map_err(|e| format!("R2 request failed: {e}"))
    }

    async fn healthcheck(&self) -> Result<(), String> {
        let response = self.request(Method::HEAD, None, None, None).await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!("R2 bucket health check failed with HTTP {}", response.status()))
        }
    }

    async fn put_file(&self, key: &str, path: &Path, content_type: &str) -> Result<(), String> {
        let _permit = self.transfer_limit.acquire().await.map_err(|_| "R2 transfer limiter is closed".to_string())?;
        let bytes = tokio::fs::read(path).await.map_err(|e| format!("Unable to read staged object: {e}"))?;
        self.put_inner(key, bytes, content_type).await
    }

    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: &str) -> Result<(), String> {
        let _permit = self.transfer_limit.acquire().await.map_err(|_| "R2 transfer limiter is closed".to_string())?;
        self.put_inner(key, bytes, content_type).await
    }

    async fn put_inner(&self, key: &str, bytes: Vec<u8>, content_type: &str) -> Result<(), String> {
        let response = self.request(Method::PUT, Some(key), Some(bytes), Some(content_type)).await?;
        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(format!("R2 upload failed with HTTP {status}: {}", truncate(&body)))
        }
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        let _permit = self.transfer_limit.acquire().await.map_err(|_| "R2 transfer limiter is closed".to_string())?;
        let response = self.request(Method::GET, Some(key), None, None).await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("R2 download failed with HTTP {status}: {}", truncate(&body)));
        }
        response.bytes().await.map(|value| Some(value.to_vec())).map_err(|e| format!("Unable to read R2 object: {e}"))
    }

    async fn delete(&self, key: &str) -> Result<(), String> {
        let response = self.request(Method::DELETE, Some(key), None, None).await?;
        if response.status().is_success() || response.status() == StatusCode::NOT_FOUND {
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(format!("R2 delete failed with HTTP {status}: {}", truncate(&body)))
        }
    }
}

fn hmac_bytes(key: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|e| format!("Unable to initialize R2 signer: {e}"))?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{:02x}", *byte)).collect()
}

fn aws_uri_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(*byte as char);
        } else {
            out.push_str(&format!("%{:02X}", *byte));
        }
    }
    out
}

fn truncate(value: &str) -> String {
    value.chars().take(300).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_keys_reject_path_traversal() {
        assert!(checked_relative_key("a/b.blob").is_ok());
        assert!(checked_relative_key("../secret").is_err());
        assert!(checked_relative_key("/absolute").is_err());
    }

    #[test]
    fn uri_encoder_preserves_s3_key_slashes() {
        assert_eq!(aws_uri_encode("a b/c+z"), "a%20b/c%2Bz");
    }
}
