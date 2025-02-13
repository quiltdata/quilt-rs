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

impl AuthIo {
    fn tokens_path(&self) -> PathBuf {
        self.dir.join(AUTH_TOKENS)
    }

    fn credentials_path(&self) -> PathBuf {
        self.dir.join(AUTH_CREDENTIALS)
    }
}
