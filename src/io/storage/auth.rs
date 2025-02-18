use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

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
        if !self.storage.exists(&self.tokens_path()).await {
            return Ok(None);
        }
        let contents = self.storage.read_file(&self.tokens_path()).await?;
        Ok(Some(serde_json::from_slice(&contents)?))
    }

    pub async fn write_tokens(&self, tokens: &Tokens) -> Res {
        let contents = serde_json::to_vec(tokens)?;
        self.storage
            .write_file(&self.tokens_path(), &contents)
            .await
    }

    pub async fn read_credentials(&self) -> Res<Option<Credentials>> {
        if !self.storage.exists(&self.credentials_path()).await {
            return Ok(None);
        }
        let contents = self.storage.read_file(&self.credentials_path()).await?;
        let credentials: Credentials = serde_json::from_slice(&contents)?;

        // Check if credentials are expired
        if credentials.expires_at <= chrono::Utc::now() {
            return Ok(None);
        }

        Ok(Some(credentials))
    }

    pub async fn get_credentials(&self) -> Res<Option<Credentials>> {
        self.read_credentials().await
    }

    pub async fn write_credentials(&self, credentials: &Credentials) -> Res {
        let contents = serde_json::to_vec(credentials)?;
        self.storage
            .write_file(&self.credentials_path(), &contents)
            .await
    }
}

impl AuthIo {
    pub fn new(storage: LocalStorage, dir: PathBuf) -> Self {
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
        let auth = AuthIo { storage, dir };

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

    /// 1. Read credentials when they don't exist yet → None
    /// 2. Write credentials → Ok
    /// 3. Read credentials are the same as written credentials
    #[tokio::test]
    async fn test_write_read_credentials() -> Res {
        let storage = MockStorage::default();
        let dir = storage.temp_dir.path().to_path_buf();
        let auth = AuthIo { storage, dir };

        let creds = auth.read_credentials().await?;
        assert!(creds.is_none());

        let test_creds = Credentials {
            access_key: "test_key".to_string(),
            secret_key: "test_secret".to_string(),
            token: "test_token".to_string(),
            expires_at: Utc::now(),
        };

        // Write credentials
        auth.write_credentials(&test_creds).await?;

        // Read them back
        let read_creds = auth.read_credentials().await?.unwrap();
        assert_eq!(read_creds.access_key, test_creds.access_key);
        assert_eq!(read_creds.secret_key, test_creds.secret_key);
        assert_eq!(read_creds.token, test_creds.token);
        assert_eq!(read_creds.expires_at, test_creds.expires_at);

        Ok(())
    }
}
