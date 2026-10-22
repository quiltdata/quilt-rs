use std::fmt;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use tracing::debug;
use tracing::trace;
use tracing::warn;

use crate::io::storage::LocalStorage;
use crate::io::storage::Storage;
use crate::io::storage::StorageExt;

use crate::Res;
use crate::paths::AUTH_CLIENT;
use crate::paths::AUTH_CREDENTIALS;
use crate::paths::AUTH_DIR;
use crate::paths::AUTH_TOKENS;

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

    /// Writes `contents` owner-only, after limiting `.auth/` and this host's
    /// directory to their owner. A directory outside `.auth/` keeps its parent
    /// as it is.
    async fn write(&self, path: &Path, contents: Vec<u8>) -> Res {
        if let Some(auth_dir) = self.dir.parent().filter(|dir| dir.ends_with(AUTH_DIR)) {
            self.storage.create_private_dir(auth_dir).await?;
        }
        self.storage.create_private_dir(&self.dir).await?;
        self.storage
            .write_private_byte_stream(path, contents.into())
            .await
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
        self.write(&tokens_path, contents).await?;

        debug!("✔️ Successfully wrote tokens: {:?}", tokens);

        Ok(())
    }

    pub async fn read_credentials(&self) -> Res<Option<Credentials>> {
        let credentials_path = self.credentials_path();
        trace!("⏳ Reading credentials from {:?}", credentials_path);

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

        // The same cache hit, one layer down. Two lines for one uneventful read.
        trace!("✔️ Successfully read valid credentials");

        Ok(Some(credentials))
    }

    pub async fn write_credentials(&self, credentials: &Credentials) -> Res {
        let credentials_path = self.credentials_path();
        debug!("⏳ Writing credentials to {:?}", credentials_path);

        let contents = serde_json::to_vec(credentials)?;
        self.write(&credentials_path, contents).await?;

        debug!("✔️ Successfully wrote credentials: {:?}", credentials);

        Ok(())
    }

    /// Remove the cached STS credentials, leaving tokens and the OAuth
    /// client untouched. The next credential read misses and re-vends —
    /// which is how a role change takes effect without logging the user out.
    /// Absent file is success: the goal is "no cached credentials", and
    /// the flush fires on the first observation of every session.
    pub async fn delete_credentials(&self) -> Res {
        let credentials_path = self.credentials_path();
        debug!("⏳ Deleting credentials at {:?}", credentials_path);

        match self.storage.remove_file(&credentials_path).await {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {
                debug!("No credentials file to delete");
            }
            Err(e) => return Err(e.into()),
        }

        debug!("✔️ Credentials deleted");
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
        self.write(&path, contents).await?;

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

    #[test(tokio::test)]
    async fn delete_credentials_removes_creds_and_keeps_tokens() -> Res {
        let storage = MockStorage::default();
        let dir = storage.temp_dir.path().to_path_buf();
        let auth = AuthIo::new(storage, dir);

        auth.write_tokens(&Tokens {
            access_token: "keep_access".to_string(),
            refresh_token: "keep_refresh".to_string(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
        })
        .await?;
        auth.write_credentials(&Credentials {
            access_key: "drop_key".to_string(),
            secret_key: "drop_secret".to_string(),
            token: "drop_token".to_string(),
            expires_at: Utc::now() + chrono::Duration::minutes(30),
        })
        .await?;

        auth.delete_credentials().await?;

        assert!(auth.read_credentials().await?.is_none());
        let tokens = auth.read_tokens().await?.expect("tokens survive the flush");
        assert_eq!(tokens.access_token, "keep_access");
        Ok(())
    }

    #[test(tokio::test)]
    async fn delete_credentials_is_idempotent_when_no_file_exists() -> Res {
        let storage = MockStorage::default();
        let dir = storage.temp_dir.path().to_path_buf();
        let auth = AuthIo::new(storage, dir);

        auth.delete_credentials().await?;
        auth.delete_credentials().await?;

        assert!(auth.read_credentials().await?.is_none());
        Ok(())
    }

    /// Every auth file is readable by its owner alone, and so are `.auth/` and
    /// the host's directory, including ones an earlier version left open.
    #[test(tokio::test)]
    #[cfg(unix)]
    async fn auth_files_and_their_directories_are_owner_only() -> Res {
        use std::os::unix::fs::PermissionsExt;

        let mode =
            |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        let data_dir = tempfile::tempdir()?;
        let auth_dir = data_dir.path().join(".auth");
        let host_dir = auth_dir.join("quilt.test");
        std::fs::create_dir_all(&host_dir)?;
        std::fs::set_permissions(&auth_dir, std::fs::Permissions::from_mode(0o755))?;
        std::fs::set_permissions(&host_dir, std::fs::Permissions::from_mode(0o755))?;

        let auth = AuthIo::new(LocalStorage::new(), host_dir.clone());
        auth.write_tokens(&Tokens {
            access_token: "access".to_string(),
            refresh_token: "refresh".to_string(),
            expires_at: Utc::now(),
        })
        .await?;
        auth.write_credentials(&Credentials {
            access_key: "key".to_string(),
            secret_key: "secret".to_string(),
            token: "token".to_string(),
            expires_at: Utc::now(),
        })
        .await?;
        auth.write_client(&OAuthClient {
            client_id: "client".to_string(),
            redirect_uri: "quilt://auth".to_string(),
        })
        .await?;

        assert_eq!(mode(&auth_dir), 0o700);
        assert_eq!(mode(&host_dir), 0o700);
        for file in [AUTH_TOKENS, AUTH_CREDENTIALS, AUTH_CLIENT] {
            assert_eq!(mode(&host_dir.join(file)), 0o600, "{file}");
        }
        Ok(())
    }

    /// Auth files kept outside `.auth/` leave the directory above theirs alone.
    #[test(tokio::test)]
    #[cfg(unix)]
    async fn auth_files_outside_the_store_leave_its_parent_alone() -> Res {
        use std::os::unix::fs::PermissionsExt;

        let parent = tempfile::tempdir()?;
        std::fs::set_permissions(parent.path(), std::fs::Permissions::from_mode(0o755))?;

        AuthIo::new(LocalStorage::new(), parent.path().join("quilt.test"))
            .write_client(&OAuthClient {
                client_id: "client".to_string(),
                redirect_uri: "quilt://auth".to_string(),
            })
            .await?;

        let mode = std::fs::metadata(parent.path())?.permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
        Ok(())
    }

    #[test]
    fn tokens_debug_redacts_secrets() {
        let tokens = Tokens {
            access_token: "secret-access".to_string(),
            refresh_token: "secret-refresh".to_string(),
            expires_at: Utc::now(),
        };
        let output = format!("{tokens:?}");
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
        let output = format!("{creds:?}");
        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("secret-key"));
        assert!(!output.contains("secret-secret"));
        assert!(!output.contains("secret-token"));
    }
}
