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
        self.storage
            .write_file(&self.tokens_path(), &contents)
            .await
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
        self.storage
            .write_file(&self.credentials_path(), &contents)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::mocks::MockStorage;
    use chrono::Utc;
    use tempfile::TempDir;

    fn setup() -> (AuthIo<MockStorage, MockRemote>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let storage = MockStorage::new();
        let remote = MockRemote::new();
        let auth = AuthIo {
            storage,
            remote,
            dir: temp_dir.path().to_path_buf(),
        };
        (auth, temp_dir)
    }

    fn make_test_tokens() -> Tokens {
        Tokens {
            access_token: "test_access".to_string(),
            refresh_token: "test_refresh".to_string(),
            expires_at: Utc::now(),
        }
    }

    fn make_test_credentials() -> Credentials {
        Credentials {
            access_key: "test_key".to_string(),
            secret_key: "test_secret".to_string(),
            token: "test_token".to_string(),
            expiry_time: Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_read_tokens_not_found() -> crate::Res<()> {
        let storage = MockStorage::default();
        let dir = storage.temp_dir.path().to_path_buf();
        let auth = AuthIo {
            storage,
            remote: MockRemote::default(),
            dir,
        };

        let tokens = auth.read_tokens().await?;
        assert!(tokens.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_write_read_tokens() -> crate::Res<()> {
        let (auth, _temp) = setup();
        let test_tokens = make_test_tokens();

        // Write tokens
        auth.write_tokens(&test_tokens).await?;

        // Read them back
        let read_tokens = auth.read_tokens().await?.unwrap();
        assert_eq!(read_tokens.access_token, test_tokens.access_token);
        assert_eq!(read_tokens.refresh_token, test_tokens.refresh_token);
        assert_eq!(read_tokens.expires_at, test_tokens.expires_at);

        Ok(())
    }

    #[tokio::test]
    async fn test_read_credentials_not_found() -> crate::Res<()> {
        let (auth, _temp) = setup();
        let creds = auth.read_credentials().await?;
        assert!(creds.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_write_read_credentials() -> crate::Res<()> {
        let (auth, _temp) = setup();
        let test_creds = make_test_credentials();

        // Write credentials
        auth.write_credentials(&test_creds).await?;

        // Read them back
        let read_creds = auth.read_credentials().await?.unwrap();
        assert_eq!(read_creds.access_key, test_creds.access_key);
        assert_eq!(read_creds.secret_key, test_creds.secret_key);
        assert_eq!(read_creds.token, test_creds.token);
        assert_eq!(read_creds.expiry_time, test_creds.expiry_time);

        Ok(())
    }
}
