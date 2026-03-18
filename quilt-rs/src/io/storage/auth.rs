use std::fmt;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use tracing::debug;
use tracing::warn;

use crate::io::storage::LocalStorage;
use crate::io::storage::Storage;
use crate::io::storage::StorageExt;

use crate::paths::AUTH_CLIENT;
use crate::paths::AUTH_CREDENTIALS;
use crate::paths::AUTH_TOKENS;
use crate::Res;

#[derive(Deserialize, Serialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

impl fmt::Debug for Tokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tokens")
            .field("expires_at", &self.expires_at)
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize, Serialize)]
pub struct Credentials {
    pub access_key: String,
    pub secret_key: String,
    pub token: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("expires_at", &self.expires_at)
            .field("access_key", &"[REDACTED]")
            .field("secret_key", &"[REDACTED]")
            .field("token", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

/// OAuth client registration data (persisted per host via DCR).
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct OAuthClient {
    pub client_id: String,
    pub redirect_uri: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct AuthIo<S: Storage = LocalStorage> {
    storage: S,
    dir: PathBuf,
}

impl<S: Storage + Sync> AuthIo<S> {
    fn client_path(&self) -> PathBuf {
        self.dir.join(AUTH_CLIENT)
    }

    fn tokens_path(&self) -> PathBuf {
        self.dir.join(AUTH_TOKENS)
    }

    fn credentials_path(&self) -> PathBuf {
        self.dir.join(AUTH_CREDENTIALS)
    }

    pub async fn read_tokens(&self) -> Res<Option<Tokens>> {
        let tokens_path = self.tokens_path();
        debug!("⏳ Reading auth tokens from {:?}", tokens_path);

        if !self.storage.exists(&tokens_path).await {
            debug!("No tokens file found");
            return Ok(None);
        }
        let bytes = self.storage.read_bytes(&tokens_path).await?;
        let tokens = serde_json::from_slice(&bytes)?;

        debug!("✔️ Successfully read tokens");

        Ok(Some(tokens))
    }

    pub async fn write_tokens(&self, tokens: &Tokens) -> Res {
        let tokens_path = self.tokens_path();
        debug!("⏳ Writing auth tokens to {:?}", tokens_path);

        let contents = serde_json::to_vec(tokens)?;
        self.storage
            .write_byte_stream(&tokens_path, contents.into())
            .await?;

        debug!("✔️ Successfully wrote tokens: {:?}", tokens);

        Ok(())
    }

    pub async fn read_credentials(&self) -> Res<Option<Credentials>> {
        let credentials_path = self.credentials_path();
        debug!("⏳ Reading credentials from {:?}", credentials_path);

        if !self.storage.exists(&credentials_path).await {
            warn!("No credentials file found");
            return Ok(None);
        }
        let bytes = self.storage.read_bytes(&credentials_path).await?;
        let credentials: Credentials = serde_json::from_slice(&bytes)?;

        // Check if credentials are expired
        if credentials.expires_at <= chrono::Utc::now() {
            warn!("❌ Credentials have expired");
            return Ok(None);
        }

        debug!("✔️ Successfully read valid credentials");

        Ok(Some(credentials))
    }

    pub async fn write_credentials(&self, credentials: &Credentials) -> Res {
        let credentials_path = self.credentials_path();
        debug!("⏳ Writing credentials to {:?}", credentials_path);

        let contents = serde_json::to_vec(credentials)?;
        self.storage
            .write_byte_stream(&credentials_path, contents.into())
            .await?;

        debug!("✔️ Successfully wrote credentials: {:?}", credentials);

        Ok(())
    }

    pub async fn read_client(&self) -> Res<Option<OAuthClient>> {
        let path = self.client_path();
        debug!("⏳ Reading OAuth client from {:?}", path);

        if !self.storage.exists(&path).await {
            debug!("No client file found");
            return Ok(None);
        }
        let bytes = self.storage.read_bytes(&path).await?;
        let client = serde_json::from_slice(&bytes)?;

        debug!("✔️ Successfully read OAuth client");

        Ok(Some(client))
    }

    pub async fn write_client(&self, client: &OAuthClient) -> Res {
        let path = self.client_path();
        debug!("⏳ Writing OAuth client to {:?}", path);

        let contents = serde_json::to_vec(client)?;
        self.storage
            .write_byte_stream(&path, contents.into())
            .await?;

        debug!("✔️ Successfully wrote OAuth client");

        Ok(())
    }
}

impl<S: Storage> AuthIo<S> {
    pub fn new(storage: S, dir: PathBuf) -> Self {
        AuthIo { storage, dir }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use chrono::Utc;

    use crate::io::storage::mocks::MockStorage;

    /// 1. Read tokens when they don't exist yet → None
    /// 2. Write tokens → Ok
    /// 3. Read tokens are the same as written tokens
    #[test(tokio::test)]
    async fn test_write_read_tokens() -> Res {
        let storage = MockStorage::default();
        let dir = storage.temp_dir.path().to_path_buf();
        let auth = AuthIo::new(storage, dir);

        let tokens = auth.read_tokens().await?;
        assert!(tokens.is_none());

        let test_tokens = Tokens {
            access_token: "test_access".to_string(),
            refresh_token: "test_refresh".to_string(),
            expires_at: Utc::now(),
        };

        // Write tokens
        auth.write_tokens(&test_tokens).await?;

        // Read them back
        let read_tokens = auth.read_tokens().await?.unwrap();
        assert_eq!(read_tokens.access_token, test_tokens.access_token);
        assert_eq!(read_tokens.refresh_token, test_tokens.refresh_token);
        assert_eq!(read_tokens.expires_at, test_tokens.expires_at);

        Ok(())
    }

    /// Tests reading and writing credentials, including expiration behavior
    #[test(tokio::test)]
    async fn test_credentials() -> Res {
        // Test non-existent credentials
        let storage = MockStorage::default();
        let dir = storage.temp_dir.path().to_path_buf();
        let auth = AuthIo::new(storage, dir);

        let creds = auth.read_credentials().await?;
        assert!(creds.is_none());

        // Test expired credentials
        let expired_creds = Credentials {
            access_key: "expired_key".to_string(),
            secret_key: "expired_secret".to_string(),
            token: "expired_token".to_string(),
            expires_at: Utc::now() - chrono::Duration::minutes(1),
        };
        auth.write_credentials(&expired_creds).await?;
        assert!(auth.read_credentials().await?.is_none());

        // Test valid credentials
        let valid_creds = Credentials {
            access_key: "test_key".to_string(),
            secret_key: "test_secret".to_string(),
            token: "test_token".to_string(),
            expires_at: Utc::now() + chrono::Duration::minutes(1),
        };
        auth.write_credentials(&valid_creds).await?;

        let read_creds = auth.read_credentials().await?.unwrap();
        assert_eq!(read_creds.access_key, valid_creds.access_key);
        assert_eq!(read_creds.secret_key, valid_creds.secret_key);
        assert_eq!(read_creds.token, valid_creds.token);
        assert_eq!(read_creds.expires_at, valid_creds.expires_at);

        Ok(())
    }

    #[test]
    fn tokens_debug_redacts_secrets() {
        let tokens = Tokens {
            access_token: "secret-access".to_string(),
            refresh_token: "secret-refresh".to_string(),
            expires_at: Utc::now(),
        };
        let output = format!("{:?}", tokens);
        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("secret-access"));
        assert!(!output.contains("secret-refresh"));
    }

    #[test]
    fn credentials_debug_redacts_secrets() {
        let creds = Credentials {
            access_key: "secret-key".to_string(),
            secret_key: "secret-secret".to_string(),
            token: "secret-token".to_string(),
            expires_at: Utc::now(),
        };
        let output = format!("{:?}", creds);
        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("secret-key"));
        assert!(!output.contains("secret-secret"));
        assert!(!output.contains("secret-token"));
    }
}
