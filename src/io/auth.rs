use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use crate::io::remote::Remote;
use crate::io::remote::RemoteS3;
use crate::io::storage::LocalStorage;
use crate::io::storage::Storage;
use crate::paths::AUTH_CREDENTIALS;
use crate::paths::AUTH_TOKENS;

#[derive(Deserialize, Serialize, Debug)]
struct Tokens {
    access_token: String,
    refresh_token: String,
    expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize, Serialize, Debug)]
struct Credentials {
    access_key: String,
    secret_key: String,
    token: String,
    expiry_time: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct AuthIo<S: Storage = LocalStorage, R: Remote = RemoteS3> {
    storage: S,
    remote: R,
    dir: PathBuf,
}

impl<S: Storage, R: Remote> AuthIo<S, R> {
    fn tokens_path(&self) -> PathBuf {
        self.dir.join(AUTH_TOKENS)
    }

    fn credentials_path(&self) -> PathBuf {
        self.dir.join(AUTH_CREDENTIALS)
    }

    pub async fn read_tokens(&self) -> crate::Res<Option<Tokens>> {
        if !self.storage.exists(&self.tokens_path()).await {
            return Ok(None);
        }
        let contents = self.storage.read_file(&self.tokens_path()).await?;
        Ok(Some(serde_json::from_slice(&contents)?))
    }

    pub async fn write_tokens(&self, tokens: &Tokens) -> crate::Res<()> {
        let contents = serde_json::to_vec(tokens)?;
        self.storage.write_file(&self.tokens_path(), &contents).await
    }

    pub async fn read_credentials(&self) -> crate::Res<Option<Credentials>> {
        if !self.storage.exists(&self.credentials_path()).await {
            return Ok(None);
        }
        let contents = self.storage.read_file(&self.credentials_path()).await?;
        Ok(Some(serde_json::from_slice(&contents)?))
    }

    pub async fn write_credentials(&self, credentials: &Credentials) -> crate::Res<()> {
        let contents = serde_json::to_vec(credentials)?;
        self.storage.write_file(&self.credentials_path(), &contents).await
    }
}
