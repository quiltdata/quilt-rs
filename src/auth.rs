use reqwest::Client as HttpClient;

use crate::io::storage::auth::AuthIo;
use crate::io::storage::auth::Credentials;
use crate::io::storage::auth::Tokens;
use crate::io::storage::LocalStorage;
use crate::paths::DomainPaths;
use crate::uri::Host;
use crate::Res;

#[derive(Debug, Clone)]
pub struct Auth {
    paths: DomainPaths,
    storage: LocalStorage,
}

impl Auth {
    pub fn new(paths: DomainPaths, storage: LocalStorage) -> Self {
        Self { paths, storage }
    }

    pub async fn login(
        &self,
        http_client: &HttpClient,
        host: &Host,
        refresh_token: String,
    ) -> Res<()> {
        // 1. Verify URL exists
        // if self.registry_url.is_empty() {
        //     return Err(Error::MissingRegistryUrl);
        // }

        // // 2. Open browser to get refresh token
        // let code_url = format!("{}/code", self.registry_url);
        // if let Err(_) = webbrowser::open(&code_url) {
        //     println!("Please visit {} to get your authentication code", code_url);
        // }

        // 3. Prompt for refresh token
        // println!("Enter the code from the webpage:");
        // let mut refresh_token = String::new();
        // std::io::stdin().read_line(&mut refresh_token)?;
        // let refresh_token = refresh_token.trim().to_string();

        // 4. Exchange refresh token for auth tokens
        let tokens = self
            .get_auth_tokens(http_client, host, &refresh_token)
            .await?;

        // 5. Cache tokens
        self.save_tokens(host, &tokens).await?;

        // 6. Get initial credentials
        self.refresh_credentials(http_client, host, &tokens.access_token)
            .await?;

        Ok(())
    }

    pub async fn get_auth_tokens(
        &self,
        http_client: &HttpClient,
        host: &Host,
        refresh_token: &str,
    ) -> Res<Tokens> {
        let response = http_client
            .post(format!("{}/api/token", host))
            .form(&[("refresh_token", refresh_token)])
            .send()
            .await?;

        Ok(response.json().await?)
    }

    async fn save_tokens(&self, host: &Host, tokens: &Tokens) -> Res<()> {
        let auth_io = AuthIo::new(self.storage.clone(), self.paths.auth_host(host));
        auth_io.write_tokens(tokens).await
    }

    async fn refresh_credentials(
        &self,
        http_client: &HttpClient,
        host: &Host,
        access_token: &str,
    ) -> Res<Credentials> {
        let response = http_client
            .post(format!("{}/api/auth/get_credentials", host))
            .bearer_auth(access_token)
            .send()
            .await?;

        let creds: Credentials = response.json().await?;

        let auth_io = AuthIo::new(self.storage.clone(), self.paths.auth_host(host));
        auth_io.write_credentials(&creds).await?;

        Ok(creds)
    }

    pub async fn get_credentials_or_refresh(
        &self,
        http_client: &HttpClient,
        host: &Host,
    ) -> Res<Credentials> {
        let auth_io = AuthIo::new(self.storage.clone(), self.paths.auth_host(host));
        match auth_io.read_credentials().await? {
            Some(creds) => Ok(creds),
            None => match auth_io.read_tokens().await? {
                Some(tokens) => {
                    self.refresh_credentials(http_client, host, &tokens.access_token)
                        .await
                }
                None => Err(crate::Error::LoginRequired),
            },
        }
    }
}
