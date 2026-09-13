use std::time::Duration;

use anyhow::{Context, Result};

use crate::config::{Config, ServerConfig};
use crate::http_source::{HttpRetryPolicy, HttpSource};
use crate::source::{DeliverySource, FilesystemSource};

/// Build the delivery source shared by the CLI and desktop frontend.
///
/// A desktop session may supply a token without persisting it. When no
/// override is present, HTTP configurations retain the CLI's environment
/// variable behavior.
pub fn source_for(
    config: &Config,
    token_override: Option<&str>,
) -> Result<Box<dyn DeliverySource>> {
    config.require_connected()?;
    match &config.server {
        ServerConfig::Filesystem { inbox_dir } => {
            Ok(Box::new(FilesystemSource::new(inbox_dir.clone())))
        }
        ServerConfig::Http {
            base_url,
            token_env,
            request_timeout_seconds,
            page_size,
            retry_max_attempts,
            retry_base_delay_milliseconds,
            retry_max_delay_milliseconds,
            allow_insecure_http,
        } => {
            let token = match token_override {
                Some(token) => token.to_owned(),
                None => std::env::var(token_env).with_context(|| {
                    format!(
                        "HTTP token environment variable {} is not set or is not valid UTF-8",
                        token_env
                    )
                })?,
            };
            let retry_policy = HttpRetryPolicy::new(
                *retry_max_attempts,
                Duration::from_millis(*retry_base_delay_milliseconds),
                Duration::from_millis(*retry_max_delay_milliseconds),
            )?;
            Ok(Box::new(HttpSource::new_with_retry(
                base_url,
                &token,
                *request_timeout_seconds,
                *page_size,
                *allow_insecure_http,
                retry_policy,
            )?))
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::config::{InitOverrides, ServerConfig};

    #[test]
    fn session_token_can_build_an_http_source_without_mutating_the_environment() {
        let root = tempdir().unwrap();
        let mut config = Config::defaults(InitOverrides {
            data_dir: Some(root.path().join("data")),
            ..Default::default()
        })
        .unwrap();
        config.server = ServerConfig::Http {
            base_url: "https://relay.example".to_owned(),
            token_env: "MIRELAY_TEST_TOKEN_THAT_SHOULD_NOT_EXIST".to_owned(),
            request_timeout_seconds: 5,
            page_size: 10,
            retry_max_attempts: 2,
            retry_base_delay_milliseconds: 1,
            retry_max_delay_milliseconds: 2,
            allow_insecure_http: false,
        };

        assert!(source_for(&config, Some("session-secret")).is_ok());
    }
}
