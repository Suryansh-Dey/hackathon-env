//! Object storage: S3 with presigned uploads (PLAN.md §6.1, §17.2), or a
//! local directory served by the dev server.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct PresignedUpload {
    pub url: String,
    pub method: String,
    /// Headers the client must send verbatim (content type, length, checksum).
    pub headers: Vec<(String, String)>,
    pub expires_in_secs: u64,
}

#[async_trait]
pub trait BlobPort: Send + Sync {
    async fn presign_put(
        &self,
        bucket: &str,
        key: &str,
        content_type: &str,
        size: u64,
        sha256_b64: Option<&str>,
    ) -> anyhow::Result<PresignedUpload>;
    async fn presign_get(&self, bucket: &str, key: &str, ttl: Duration) -> anyhow::Result<String>;
    async fn get(&self, bucket: &str, key: &str) -> anyhow::Result<Vec<u8>>;
    async fn put(&self, bucket: &str, key: &str, bytes: Vec<u8>, content_type: &str) -> anyhow::Result<()>;
    async fn exists(&self, bucket: &str, key: &str) -> anyhow::Result<bool>;
}

pub struct S3Blobs {
    client: aws_sdk_s3::Client,
}

impl S3Blobs {
    pub fn new(client: aws_sdk_s3::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl BlobPort for S3Blobs {
    async fn presign_put(
        &self,
        bucket: &str,
        key: &str,
        content_type: &str,
        size: u64,
        sha256_b64: Option<&str>,
    ) -> anyhow::Result<PresignedUpload> {
        use aws_sdk_s3::presigning::PresigningConfig;
        let ttl = Duration::from_secs(600);
        let mut req = self.client.put_object().bucket(bucket).key(key).content_type(content_type).content_length(size as i64);
        if let Some(c) = sha256_b64 {
            req = req.checksum_sha256(c);
        }
        let p = req.presigned(PresigningConfig::expires_in(ttl)?).await?;
        Ok(PresignedUpload {
            url: p.uri().to_owned(),
            method: p.method().to_owned(),
            headers: p.headers().map(|(k, v)| (k.to_owned(), v.to_owned())).collect(),
            expires_in_secs: ttl.as_secs(),
        })
    }

    async fn presign_get(&self, bucket: &str, key: &str, ttl: Duration) -> anyhow::Result<String> {
        use aws_sdk_s3::presigning::PresigningConfig;
        Ok(self.client.get_object().bucket(bucket).key(key).presigned(PresigningConfig::expires_in(ttl)?).await?.uri().to_owned())
    }

    async fn get(&self, bucket: &str, key: &str) -> anyhow::Result<Vec<u8>> {
        let out = self.client.get_object().bucket(bucket).key(key).send().await?;
        Ok(out.body.collect().await?.into_bytes().to_vec())
    }

    async fn put(&self, bucket: &str, key: &str, bytes: Vec<u8>, content_type: &str) -> anyhow::Result<()> {
        self.client.put_object().bucket(bucket).key(key).content_type(content_type).body(bytes.into()).send().await?;
        Ok(())
    }

    async fn exists(&self, bucket: &str, key: &str) -> anyhow::Result<bool> {
        match self.client.head_object().bucket(bucket).key(key).send().await {
            Ok(_) => Ok(true),
            Err(e) if e.as_service_error().is_some_and(|s| s.is_not_found()) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
}

/// Local directory store; URLs point at the dev server's `/dev/blobs/...`.
pub struct LocalBlobs {
    root: PathBuf,
    base_url: String,
}

impl LocalBlobs {
    pub fn new(root: PathBuf, base_url: &str) -> Self {
        Self { root, base_url: base_url.trim_end_matches('/').to_owned() }
    }

    /// Rejects path traversal; keys are `[A-Za-z0-9_./-]` without `..`.
    pub fn path(&self, bucket: &str, key: &str) -> anyhow::Result<PathBuf> {
        let ok = |s: &str| !s.is_empty() && !s.contains("..") && s.chars().all(|c| c.is_ascii_alphanumeric() || "_./-".contains(c));
        if !ok(bucket) || !ok(key) || key.starts_with('/') {
            anyhow::bail!("invalid blob key");
        }
        Ok(self.root.join(bucket).join(key))
    }
}

#[async_trait]
impl BlobPort for LocalBlobs {
    async fn presign_put(
        &self,
        bucket: &str,
        key: &str,
        content_type: &str,
        size: u64,
        _sha: Option<&str>,
    ) -> anyhow::Result<PresignedUpload> {
        self.path(bucket, key)?;
        Ok(PresignedUpload {
            url: format!("{}/dev/blobs/{bucket}/{key}", self.base_url),
            method: "PUT".into(),
            headers: vec![("content-type".into(), content_type.into()), ("content-length".into(), size.to_string())],
            expires_in_secs: 600,
        })
    }

    async fn presign_get(&self, bucket: &str, key: &str, _ttl: Duration) -> anyhow::Result<String> {
        self.path(bucket, key)?;
        Ok(format!("{}/dev/blobs/{bucket}/{key}", self.base_url))
    }

    async fn get(&self, bucket: &str, key: &str) -> anyhow::Result<Vec<u8>> {
        Ok(tokio::fs::read(self.path(bucket, key)?).await?)
    }

    async fn put(&self, bucket: &str, key: &str, bytes: Vec<u8>, _ct: &str) -> anyhow::Result<()> {
        let p = self.path(bucket, key)?;
        if let Some(parent) = p.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        Ok(tokio::fs::write(p, bytes).await?)
    }

    async fn exists(&self, bucket: &str, key: &str) -> anyhow::Result<bool> {
        Ok(tokio::fs::try_exists(self.path(bucket, key)?).await?)
    }
}
