//! Shared, bounded connection setup for the desktop and Android JNI client.
//! No redirects, token-bearing URLs, retries of mutations, or untrusted errors.
use crate::protocol::*;
use anyhow::{Context, Result, bail, ensure};
use reqwest::{
    Method, Url,
    blocking::Client,
    header::{AUTHORIZATION, HeaderValue},
};
use serde::{Serialize, de::DeserializeOwned};
use std::{io::Read, time::Duration};

pub struct PairingClient {
    base: Url,
    client: Client,
    token: HeaderValue,
}

impl PairingClient {
    pub fn new(base: &str, token: &str, insecure: bool) -> Result<Self> {
        ensure!(base.len() <= 2048, "Server URL is too long.");
        let mut base = Url::parse(base).context("Invalid server URL.")?;
        ensure!(
            base.host().is_some()
                && base.username().is_empty()
                && base.password().is_none()
                && base.query().is_none()
                && base.fragment().is_none(),
            "Use a server URL without credentials, query or fragment."
        );
        ensure!(
            base.scheme() == "https" || (insecure && base.scheme() == "http"),
            "HTTPS is required."
        );
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        ensure!(
            !token.is_empty() && token.len() <= 4096 && token.bytes().all(|b| b.is_ascii_graphic()),
            "Enter a valid credential."
        );
        let mut token =
            HeaderValue::from_str(&format!("Bearer {token}")).context("Invalid credential.")?;
        token.set_sensitive(true);
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            base,
            client,
            token,
        })
    }

    fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&impl Serialize>,
    ) -> Result<T> {
        self.request_limited(method, path, body, 16 * 1024)
    }

    pub(crate) fn request_limited<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&impl Serialize>,
        limit: usize,
    ) -> Result<T> {
        let url = self.base.join(path)?;
        let mut request = self
            .client
            .request(method, url)
            .header(AUTHORIZATION, self.token.clone())
            .header(PROTOCOL_HEADER, PROTOCOL_VERSION.to_string());
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().map_err(|_| anyhow::anyhow!("Connection failed. Check the server address, trusted HTTPS certificate and network, then retry."))?;
        match response.status().as_u16() {
            200..=299 => (),
            401 => bail!(
                "Credential or pairing code is invalid, expired, or already used by another device."
            ),
            403 => bail!("This credential does not have the required Folder permission."),
            404 => bail!(
                "This server does not support Folder pairing, or the Folder address is incorrect."
            ),
            409 => {
                bail!("Folder state changed or needs confirmation. Refresh before retrying.")
            }
            410 => bail!("This Folder has been disconnected. Create a new Folder to reconnect."),
            426 => bail!("Server and app protocol versions are incompatible."),
            _ => bail!("The server could not complete setup. No automatic retry was made."),
        }
        let mut bytes = Vec::new();
        response
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .context("Incomplete setup response.")?;
        ensure!(bytes.len() <= limit, "Setup response is too large.");
        serde_json::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("Invalid setup response from server."))
    }

    pub fn create_folder(&self, name: &str) -> Result<FolderCreated> {
        ensure!(
            !name.trim().is_empty() && name.len() <= 128 && !name.chars().any(char::is_control),
            "Enter a Folder name (up to 128 bytes)."
        );
        let created: FolderCreated = self.request(
            Method::POST,
            "api/v1/folders",
            Some(&CreateFolderRequest {
                name: name.trim().into(),
            }),
        )?;
        validate_code(&created.pairing_code)?;
        ensure!(
            created
                .pairing_code
                .starts_with(&format!("{}.", created.folder_id))
                && valid_secret(&created.receiver_token),
            "Invalid Folder credentials from server."
        );
        Ok(created)
    }

    pub fn claim(&self, code: &str, sender_token: &str) -> Result<FolderHandshake> {
        let id = validate_code(code)?;
        ensure!(valid_secret(sender_token), "Invalid sender credential.");
        let info: FolderHandshake = self.request(
            Method::POST,
            "api/v1/pairings/claim",
            Some(&ClaimFolderRequest {
                pairing_code: code.into(),
                sender_token: sender_token.into(),
            }),
        )?;
        validate_handshake(&info)?;
        ensure!(
            info.folder_id == id && info.role == "sender",
            "Server returned a different Folder identity or permission."
        );
        Ok(info)
    }

    pub fn handshake(&self) -> Result<FolderHandshake> {
        let info: FolderHandshake = self.request(Method::GET, "api/v1/handshake", None::<&()>)?;
        validate_handshake(&info)?;
        ensure!(
            self.base
                .path()
                .ends_with(&format!("/f/{}/", info.folder_id)),
            "Server returned a different Folder identity."
        );
        Ok(info)
    }

    pub fn confirm(&self, verification: &str) -> Result<FolderHandshake> {
        let info: FolderHandshake = self.request(
            Method::POST,
            "api/v1/pairing/confirm",
            Some(&ConfirmFolderRequest {
                verification: verification.into(),
            }),
        )?;
        validate_handshake(&info)?;
        ensure!(
            info.state == "ready"
                && info.role == "receiver"
                && self
                    .base
                    .path()
                    .ends_with(&format!("/f/{}/", info.folder_id)),
            "Pairing was not confirmed for this receiver."
        );
        Ok(info)
    }

    pub fn renew(&self) -> Result<FolderInvitation> {
        let invite: FolderInvitation =
            self.request(Method::POST, "api/v1/pairing/renew", None::<&()>)?;
        let id = validate_code(&invite.pairing_code)?;
        ensure!(
            self.base.path().ends_with(&format!("/f/{id}/")),
            "Server returned a different Folder invitation."
        );
        Ok(invite)
    }

    pub fn disconnect(&self) -> Result<FolderHandshake> {
        let info: FolderHandshake =
            self.request(Method::POST, "api/v1/pairing/disconnect", None::<&()>)?;
        validate_handshake(&info)?;
        ensure!(
            info.state == "disconnected"
                && self
                    .base
                    .path()
                    .ends_with(&format!("/f/{}/", info.folder_id)),
            "Server did not confirm disconnection of this Folder. Keep the credential and retry."
        );
        Ok(info)
    }

    /// Legacy configurations remain usable, but are not represented as paired.
    pub fn verify_receiver(&self) -> Result<Option<FolderHandshake>> {
        if self.base.path().contains("/f/") {
            let info = self.handshake()?;
            ensure!(
                info.role == "receiver",
                "Use a receiver credential on Linux."
            );
            Ok(Some(info))
        } else {
            let index: DeliveryIndex<DeliveryDescriptor> = self.request(
                Method::GET,
                "api/v1/deliveries?status=pending&limit=1",
                None::<&()>,
            )?;
            ensure!(
                index.schema_version == PROTOCOL_VERSION,
                "Incompatible server protocol."
            );
            Ok(None)
        }
    }
}

fn valid_secret(secret: &str) -> bool {
    secret.len() == 64 && secret.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn validate_code(code: &str) -> Result<&str> {
    let (id, secret) = code
        .split_once('.')
        .context("Paste the full pairing code from Linux.")?;
    ensure!(
        uuid::Uuid::parse_str(id)
            .ok()
            .is_some_and(|v| v.to_string() == id)
            && valid_secret(secret),
        "Invalid pairing code."
    );
    Ok(id)
}

fn validate_handshake(info: &FolderHandshake) -> Result<()> {
    ensure!(
        info.schema_version == PROTOCOL_VERSION
            && uuid::Uuid::parse_str(&info.folder_id)
                .ok()
                .is_some_and(|v| v.to_string() == info.folder_id)
            && matches!(info.role.as_str(), "receiver" | "sender")
            && matches!(
                info.state.as_str(),
                "ready" | "awaiting_peer" | "awaiting_confirmation" | "disconnected"
            )
            && !info.name.trim().is_empty()
            && info.name.len() <= 128
            && !info.name.chars().any(char::is_control)
            && info.max_file_size_bytes > 0
            && match (&info.verification, info.state.as_str()) {
                (None, "awaiting_peer" | "disconnected") => true,
                (Some(v), "ready" | "awaiting_confirmation") =>
                    v.len() == 12 && v.bytes().all(|b| b.is_ascii_hexdigit()),
                _ => false,
            },
        "Invalid Folder handshake."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use serde_json::json;
    const ID: &str = "00000000-0000-4000-8000-000000000001";

    #[test]
    fn setup_redirects_never_forward_credentials() {
        let source = MockServer::start();
        let destination = MockServer::start();
        let escaped = destination.mock(|when, then| {
            when.any_request();
            then.status(200);
        });
        source.mock(|when, then| {
            when.path(format!("/f/{ID}/api/v1/handshake"));
            then.status(302)
                .header("Location", destination.url("/capture"));
        });
        let client = PairingClient::new(
            &source.url(format!("/f/{ID}")),
            "private-test-credential",
            true,
        )
        .unwrap();
        assert!(client.handshake().is_err());
        escaped.assert_calls(0);
    }

    #[test]
    fn handshake_rejects_wrong_folder_role_state_and_oversized_body() {
        for body in [json!({"schema_version":1,"folder_id":"00000000-0000-4000-8000-000000000002","name":"Test","role":"receiver","state":"ready","verification":"012345abcdef","max_file_size_bytes":1024}).to_string(),
            json!({"schema_version":1,"folder_id":ID,"name":"Test","role":"admin","state":"ready","verification":"012345abcdef","max_file_size_bytes":1024}).to_string(),
            json!({"schema_version":1,"folder_id":ID,"name":"Test","role":"receiver","state":"ready","verification":null,"max_file_size_bytes":1024}).to_string(),
            " ".repeat(17000)] {
            let server=MockServer::start();
            server.mock(|when,then|{when.path(format!("/f/{ID}/api/v1/handshake"));then.status(200).body(body);});
            assert!(PairingClient::new(&server.url(format!("/f/{ID}")),"test-token",true).unwrap().handshake().is_err());
        }
    }

    #[test]
    fn disconnect_rejects_unconfirmed_or_mismatched_receipts_without_retry() {
        for (folder_id, state, verification) in [
            (ID, "ready", Some("012345abcdef")),
            ("00000000-0000-4000-8000-000000000002", "disconnected", None),
            (ID, "disconnected", Some("012345abcdef")),
        ] {
            let server = MockServer::start();
            let request = server.mock(|when, then| {
                when.method(httpmock::Method::POST).path(format!("/f/{ID}/api/v1/pairing/disconnect"));
                then.status(200).json_body(json!({"schema_version":1,"folder_id":folder_id,"name":"Test","role":"receiver","state":state,"verification":verification,"max_file_size_bytes":1024}));
            });
            assert!(
                PairingClient::new(&server.url(format!("/f/{ID}")), "test-token", true)
                    .unwrap()
                    .disconnect()
                    .is_err()
            );
            request.assert_calls(1);
        }
    }

    #[test]
    fn error_body_cannot_echo_credentials_and_mutation_is_not_retried() {
        let server = MockServer::start();
        let secret = "private-test-credential";
        let request = server.mock(|when, then| {
            when.path("/api/v1/folders");
            then.status(500).body(secret);
        });
        let error = PairingClient::new(&server.url("/"), secret, true)
            .unwrap()
            .create_folder("Test")
            .err()
            .unwrap();
        assert!(!format!("{error:#}").contains(secret));
        request.assert_calls(1);
    }
}
