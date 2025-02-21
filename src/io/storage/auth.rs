use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use tracing::debug;

use crate::io::storage::LocalStorage;
use crate::io::storage::Storage;
use crate::paths::AUTH_CREDENTIALS;
use crate::paths::AUTH_TOKENS;
use crate::Res;

#[derive(Deserialize, Serialize, Debug)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct Credentials {
    pub access_key: String,
    pub secret_key: String,
    pub token: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct AuthIo<S: Storage = LocalStorage> {
    storage: S,
    dir: PathBuf,
}

impl<S: Storage> AuthIo<S> {
    fn tokens_path(&self) -> PathBuf {
        self.dir.join(AUTH_TOKENS)
    }

    fn credentials_path(&self) -> PathBuf {
        self.dir.join(AUTH_CREDENTIALS)
    }

    pub async fn read_tokens(&self) -> Res<Option<Tokens>> {
        debug!("Reading auth tokens from {:?}", self.tokens_path());
        if !self.storage.exists(&self.tokens_path()).await {
            debug!("No tokens file found");
            return Ok(None);
        }
        let contents = self.storage.read_file(&self.tokens_path()).await?;
        let tokens = serde_json::from_slice(&contents)?;
        debug!("Successfully read tokens");
        Ok(Some(tokens))
    }

    pub async fn write_tokens(&self, tokens: &Tokens) -> Res {
        debug!("Writing auth tokens to {:?}", self.tokens_path());
        let contents = serde_json::to_vec(tokens)?;
        self.storage
            .write_file(&self.tokens_path(), &contents)
            .await?;
        debug!("Successfully wrote tokens: {:?}", tokens);
        Ok(())
    }

    pub async fn read_credentials(&self) -> Res<Option<Credentials>> {
        debug!("Reading credentials from {:?}", self.credentials_path());
        if !self.storage.exists(&self.credentials_path()).await {
            debug!("No credentials file found");
            return Ok(None);
        }
        let contents = self.storage.read_file(&self.credentials_path()).await?;
        let credentials: Credentials = serde_json::from_slice(&contents)?;

        // Check if credentials are expired
        if credentials.expires_at <= chrono::Utc::now() {
            debug!("Credentials have expired");
            return Ok(None);
        }

        debug!("Successfully read valid credentials");
        Ok(Some(credentials))
    }

    pub async fn write_credentials(&self, credentials: &Credentials) -> Res {
        debug!("Writing credentials to {:?}", self.credentials_path());
        let contents = serde_json::to_vec(credentials)?;
        self.storage
            .write_file(&self.credentials_path(), &contents)
            .await?;
        debug!("Successfully wrote credentials: {:?}", credentials);
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

    use chrono::Utc;

    use crate::io::storage::mocks::MockStorage;

    /// 1. Read tokens when they don't exist yet → None
    /// 2. Write tokens → Ok
    /// 3. Read tokens are the same as written tokens
    #[tokio::test]
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
    #[tokio::test]
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
}
