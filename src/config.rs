use std::env;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::fsutil::{atomic_write, create_dir_all_durable, read_limited};

pub const CONFIG_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_MAX_FILE_SIZE_BYTES: u64 = 100 * 1024 * 1024;
pub const MAX_WALLPAPER_TIMEOUT_SECONDS: u64 = 60 * 60;
pub const DEFAULT_HTTP_TOKEN_ENV: &str = "MIRELAY_TOKEN";
pub const DEFAULT_HTTP_REQUEST_TIMEOUT_SECONDS: u64 = 120;
pub const DEFAULT_HTTP_PAGE_SIZE: u32 = 50;
pub const DEFAULT_HTTP_RETRY_MAX_ATTEMPTS: u32 = 3;
pub const DEFAULT_HTTP_RETRY_BASE_DELAY_MILLISECONDS: u64 = 250;
pub const DEFAULT_HTTP_RETRY_MAX_DELAY_MILLISECONDS: u64 = 5_000;
pub const MAX_HTTP_PAGE_SIZE: u32 = 100;
pub const MAX_HTTP_RETRY_ATTEMPTS: u32 = 10;
pub const MAX_HTTP_RETRY_DELAY_MILLISECONDS: u64 = 60_000;
const MAX_CONFIG_SIZE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    #[default]
    Connected,
    DisconnectPending,
    Disconnected,
    ExitPending,
    ExitLocalCleaned,
    Exited,
}
impl ConnectionState {
    pub fn is_connected(&self) -> bool {
        *self == Self::Connected
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "ConnectionState::is_connected")]
    pub connection_state: ConnectionState,
    /// Explicit opt-in. Older readers reject this unknown field rather than
    /// accidentally running the delivery-only pipeline in a managed directory.
    #[serde(default, skip_serializing_if = "is_false")]
    pub directory_sync: bool,
    pub device_id: String,
    pub server: ServerConfig,
    pub storage: StorageConfig,
    pub wallpaper: WallpaperConfig,
    pub limits: LimitsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ServerConfig {
    Filesystem {
        inbox_dir: PathBuf,
    },
    Http {
        base_url: String,
        token_env: String,
        #[serde(default = "default_http_request_timeout_seconds")]
        request_timeout_seconds: u64,
        #[serde(default = "default_http_page_size")]
        page_size: u32,
        #[serde(default = "default_http_retry_max_attempts")]
        retry_max_attempts: u32,
        #[serde(default = "default_http_retry_base_delay_milliseconds")]
        retry_base_delay_milliseconds: u64,
        #[serde(default = "default_http_retry_max_delay_milliseconds")]
        retry_max_delay_milliseconds: u64,
        #[serde(default)]
        allow_insecure_http: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StorageConfig {
    pub library_dir: PathBuf,
    pub state_file: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WallpaperConfig {
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default = "default_wallpaper_timeout_seconds")]
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LimitsConfig {
    pub max_file_size_bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct InitOverrides {
    pub data_dir: Option<PathBuf>,
    pub library_dir: Option<PathBuf>,
    pub inbox_dir: Option<PathBuf>,
}

impl Config {
    pub fn require_connected(&self) -> Result<()> {
        anyhow::ensure!(
            self.connection_state.is_connected(),
            "This Folder is disconnected or awaiting disconnection. Retry disconnect in Folder settings; create a new Folder to reconnect."
        );
        Ok(())
    }
    pub fn defaults(overrides: InitOverrides) -> Result<Self> {
        let data_dir = match overrides.data_dir {
            Some(path) => absolute_path(path)?,
            None => default_data_dir()?,
        };
        let inbox_dir = match overrides.inbox_dir {
            Some(path) => absolute_path(path)?,
            None => data_dir.join("mock-inbox"),
        };
        let library_dir = match overrides.library_dir {
            Some(path) => absolute_path(path)?,
            None => data_dir.join("library"),
        };

        Ok(Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            connection_state: ConnectionState::Connected,
            directory_sync: false,
            device_id: Uuid::new_v4().to_string(),
            server: ServerConfig::Filesystem { inbox_dir },
            storage: StorageConfig {
                library_dir,
                state_file: data_dir.join("state.json"),
            },
            wallpaper: WallpaperConfig {
                command: vec![],
                timeout_seconds: default_wallpaper_timeout_seconds(),
            },
            limits: LimitsConfig {
                max_file_size_bytes: DEFAULT_MAX_FILE_SIZE_BYTES,
            },
        })
    }

    pub fn load(path: &Path) -> Result<Self> {
        let raw = read_limited(path, MAX_CONFIG_SIZE_BYTES)
            .with_context(|| format!("failed to read config {}", path.display()))?;
        let raw = String::from_utf8(raw)
            .with_context(|| format!("config {} is not valid UTF-8", path.display()))?;
        let config: Self = toml::from_str(&raw)
            .with_context(|| format!("failed to parse config {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn save(&self, path: &Path, force: bool) -> Result<()> {
        self.validate()?;
        if path.exists() && !force {
            bail!(
                "config already exists at {}; pass --force to replace it",
                path.display()
            );
        }

        let mut raw = String::from(
            "# MiRelay Linux CLI configuration.\n# Replace wallpaper.command with the command used by your desktop.\n",
        );
        raw.push_str(&toml::to_string_pretty(self).context("failed to serialize config")?);
        atomic_write(path, raw.as_bytes())
    }

    pub fn ensure_directories(&self) -> Result<()> {
        if let ServerConfig::Filesystem { inbox_dir } = &self.server {
            create_dir_all_durable(inbox_dir)?;
        }
        if self.directory_sync {
            anyhow::ensure!(
                self.storage.library_dir.is_dir(),
                "Choose an existing directory for directory sync."
            );
        } else {
            create_dir_all_durable(&self.storage.library_dir)?;
        }
        if let Some(parent) = self.storage.state_file.parent() {
            create_dir_all_durable(parent)?;
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            bail!(
                "unsupported config schema version {}; expected {}",
                self.schema_version,
                CONFIG_SCHEMA_VERSION
            );
        }
        if self.device_id.is_empty()
            || self.device_id.len() > 128
            || !self
                .device_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            bail!("device_id must contain 1-128 ASCII letters, digits, '-' or '_'");
        }
        match &self.server {
            ServerConfig::Filesystem { inbox_dir } => {
                if inbox_dir.as_os_str().is_empty() {
                    bail!("server.inbox_dir must not be empty");
                }
                if !inbox_dir.is_absolute() {
                    bail!("server.inbox_dir must be an absolute path");
                }
            }
            ServerConfig::Http { .. } => validate_http_server(&self.server)?,
        }
        if self.directory_sync {
            let ServerConfig::Http { base_url, .. } = &self.server else {
                bail!("Directory sync requires a paired HTTP Folder.");
            };
            let url = reqwest::Url::parse(base_url)?;
            let scope = url.path().trim_end_matches('/').strip_prefix("/f/");
            anyhow::ensure!(
                scope.is_some_and(
                    |id| Uuid::parse_str(id).is_ok_and(|value| value.to_string() == id)
                ),
                "Directory sync requires a scoped /f/<Folder ID> URL."
            );
            anyhow::ensure!(
                self.wallpaper.command.is_empty(),
                "Directory sync does not run wallpaper commands."
            );
            anyhow::ensure!(
                self.limits.max_file_size_bytes == DEFAULT_MAX_FILE_SIZE_BYTES,
                "Directory sync currently requires the 100 MiB file limit."
            );
        }
        if self.storage.library_dir.as_os_str().is_empty() {
            bail!("storage.library_dir must not be empty");
        }
        if !self.storage.library_dir.is_absolute() {
            bail!("storage.library_dir must be an absolute path");
        }
        if self.storage.state_file.as_os_str().is_empty() {
            bail!("storage.state_file must not be empty");
        }
        if !self.storage.state_file.is_absolute() {
            bail!("storage.state_file must be an absolute path");
        }
        if self.limits.max_file_size_bytes == 0 {
            bail!("limits.max_file_size_bytes must be greater than zero");
        }
        validate_wallpaper_command(&self.wallpaper.command)?;
        if self.wallpaper.timeout_seconds == 0 {
            bail!("wallpaper.timeout_seconds must be greater than zero");
        }
        if self.wallpaper.timeout_seconds > MAX_WALLPAPER_TIMEOUT_SECONDS {
            bail!(
                "wallpaper.timeout_seconds must not exceed {}",
                MAX_WALLPAPER_TIMEOUT_SECONDS
            );
        }
        Ok(())
    }
}

fn is_false(value: &bool) -> bool {
    !value
}

/// Kept outside the projection and separate from delivery-only state.
pub fn directory_state_dir(config: &Config) -> PathBuf {
    let mut path = config.storage.state_file.as_os_str().to_owned();
    path.push(".directory");
    PathBuf::from(path)
}

fn default_wallpaper_timeout_seconds() -> u64 {
    30
}

fn default_http_request_timeout_seconds() -> u64 {
    DEFAULT_HTTP_REQUEST_TIMEOUT_SECONDS
}

fn default_http_page_size() -> u32 {
    DEFAULT_HTTP_PAGE_SIZE
}

fn default_http_retry_max_attempts() -> u32 {
    DEFAULT_HTTP_RETRY_MAX_ATTEMPTS
}

fn default_http_retry_base_delay_milliseconds() -> u64 {
    DEFAULT_HTTP_RETRY_BASE_DELAY_MILLISECONDS
}

fn default_http_retry_max_delay_milliseconds() -> u64 {
    DEFAULT_HTTP_RETRY_MAX_DELAY_MILLISECONDS
}

fn validate_http_server(server: &ServerConfig) -> Result<()> {
    let ServerConfig::Http {
        base_url,
        token_env,
        request_timeout_seconds,
        page_size,
        retry_max_attempts,
        retry_base_delay_milliseconds,
        retry_max_delay_milliseconds,
        allow_insecure_http,
    } = server
    else {
        unreachable!("HTTP validation requires an HTTP server configuration");
    };
    if base_url.len() > 2048 || base_url.chars().any(char::is_control) {
        bail!("server.base_url is too long or contains control characters");
    }
    let url = reqwest::Url::parse(base_url).context("server.base_url is not a valid URL")?;
    if url.host().is_none() {
        bail!("server.base_url must include a host");
    }
    match url.scheme() {
        "https" => {}
        "http" if *allow_insecure_http => {}
        "http" => {
            bail!(
                "plain HTTP is disabled; use HTTPS or explicitly set allow_insecure_http = true for local development"
            )
        }
        scheme => bail!("unsupported server URL scheme {scheme:?}; expected https"),
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("server.base_url must not contain credentials");
    }
    if url.query().is_some() || url.fragment().is_some() {
        bail!("server.base_url must not contain a query string or fragment");
    }
    if token_env.is_empty()
        || token_env.len() > 128
        || !token_env
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        bail!("server.token_env must be a valid environment variable name");
    }
    if *request_timeout_seconds == 0 || *request_timeout_seconds > 60 * 60 {
        bail!("server.request_timeout_seconds must be between 1 and 3600");
    }
    if *page_size == 0 || *page_size > MAX_HTTP_PAGE_SIZE {
        bail!(
            "server.page_size must be between 1 and {}",
            MAX_HTTP_PAGE_SIZE
        );
    }
    if *retry_max_attempts == 0 || *retry_max_attempts > MAX_HTTP_RETRY_ATTEMPTS {
        bail!(
            "server.retry_max_attempts must be between 1 and {}",
            MAX_HTTP_RETRY_ATTEMPTS
        );
    }
    if *retry_base_delay_milliseconds == 0
        || *retry_base_delay_milliseconds > MAX_HTTP_RETRY_DELAY_MILLISECONDS
    {
        bail!(
            "server.retry_base_delay_milliseconds must be between 1 and {}",
            MAX_HTTP_RETRY_DELAY_MILLISECONDS
        );
    }
    if retry_max_delay_milliseconds < retry_base_delay_milliseconds
        || *retry_max_delay_milliseconds > MAX_HTTP_RETRY_DELAY_MILLISECONDS
    {
        bail!(
            "server.retry_max_delay_milliseconds must be between server.retry_base_delay_milliseconds and {}",
            MAX_HTTP_RETRY_DELAY_MILLISECONDS
        );
    }
    Ok(())
}

pub fn validate_wallpaper_command(command: &[String]) -> Result<()> {
    if command.is_empty() {
        return Ok(());
    }
    if command[0].trim().is_empty() {
        bail!("wallpaper.command executable must not be empty");
    }
    if command[0].contains("{path}") {
        bail!("wallpaper.command executable cannot contain {{path}}");
    }
    if !command.iter().skip(1).any(|part| part == "{path}") {
        bail!("wallpaper.command must contain a standalone {{path}} argument");
    }
    if command
        .iter()
        .skip(1)
        .any(|part| part.contains("{path}") && part != "{path}")
    {
        bail!("{{path}} must be a standalone wallpaper.command argument");
    }
    Ok(())
}

pub fn default_config_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os("XDG_CONFIG_HOME") {
        return absolute_path(PathBuf::from(path).join("mirelay/config.toml"));
    }
    absolute_path(home_dir()?.join(".config/mirelay/config.toml"))
}

pub fn default_data_dir() -> Result<PathBuf> {
    if let Some(path) = env::var_os("XDG_DATA_HOME") {
        return absolute_path(PathBuf::from(path).join("mirelay"));
    }
    absolute_path(home_dir()?.join(".local/share/mirelay"))
}

fn home_dir() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .context("cannot determine the home directory; set HOME or the relevant XDG variable")
}

fn absolute_path(path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(env::current_dir()
            .context("failed to determine the current directory")?
            .join(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wallpaper_path_must_be_a_standalone_argument() {
        let command = vec!["setter".into(), "--image={path}".into()];
        assert!(validate_wallpaper_command(&command).is_err());

        let command = vec!["setter".into(), "--image".into(), "{path}".into()];
        assert!(validate_wallpaper_command(&command).is_ok());
    }

    #[test]
    fn wallpaper_timeout_has_a_safety_ceiling() {
        let root = tempfile::tempdir().unwrap();
        let mut config = Config::defaults(InitOverrides {
            data_dir: Some(root.path().to_path_buf()),
            ..Default::default()
        })
        .unwrap();
        config.wallpaper.timeout_seconds = MAX_WALLPAPER_TIMEOUT_SECONDS + 1;
        assert!(config.validate().is_err());
    }

    #[test]
    fn plain_http_requires_explicit_local_development_opt_in() {
        let root = tempfile::tempdir().unwrap();
        let mut config = Config::defaults(InitOverrides {
            data_dir: Some(root.path().to_path_buf()),
            ..Default::default()
        })
        .unwrap();
        config.server = ServerConfig::Http {
            base_url: "http://127.0.0.1:8080".into(),
            token_env: DEFAULT_HTTP_TOKEN_ENV.into(),
            request_timeout_seconds: DEFAULT_HTTP_REQUEST_TIMEOUT_SECONDS,
            page_size: DEFAULT_HTTP_PAGE_SIZE,
            retry_max_attempts: DEFAULT_HTTP_RETRY_MAX_ATTEMPTS,
            retry_base_delay_milliseconds: DEFAULT_HTTP_RETRY_BASE_DELAY_MILLISECONDS,
            retry_max_delay_milliseconds: DEFAULT_HTTP_RETRY_MAX_DELAY_MILLISECONDS,
            allow_insecure_http: false,
        };

        assert!(config.validate().is_err());
        if let ServerConfig::Http {
            allow_insecure_http,
            ..
        } = &mut config.server
        {
            *allow_insecure_http = true;
        }
        assert!(config.validate().is_ok());
    }

    #[test]
    fn http_configuration_round_trips_without_a_secret() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.toml");
        let mut config = Config::defaults(InitOverrides {
            data_dir: Some(root.path().join("data")),
            ..Default::default()
        })
        .unwrap();
        config.server = ServerConfig::Http {
            base_url: "https://mirelay.example/api-prefix".into(),
            token_env: "MIRELAY_PRIVATE_TOKEN".into(),
            request_timeout_seconds: 45,
            page_size: 25,
            retry_max_attempts: 4,
            retry_base_delay_milliseconds: 100,
            retry_max_delay_milliseconds: 2_000,
            allow_insecure_http: false,
        };

        config.save(&path, false).unwrap();
        assert_eq!(Config::load(&path).unwrap(), config);
        let raw = std::fs::read_to_string(path).unwrap();
        assert!(raw.contains("token_env = \"MIRELAY_PRIVATE_TOKEN\""));
        assert!(!raw.contains("Bearer"));
    }

    #[test]
    fn legacy_http_configuration_receives_retry_defaults() {
        let server: ServerConfig = toml::from_str(
            r#"
kind = "http"
base_url = "https://mirelay.example"
token_env = "MIRELAY_TOKEN"
request_timeout_seconds = 120
page_size = 50
allow_insecure_http = false
"#,
        )
        .unwrap();

        let ServerConfig::Http {
            retry_max_attempts,
            retry_base_delay_milliseconds,
            retry_max_delay_milliseconds,
            ..
        } = server
        else {
            panic!("expected HTTP server configuration");
        };
        assert_eq!(retry_max_attempts, DEFAULT_HTTP_RETRY_MAX_ATTEMPTS);
        assert_eq!(
            retry_base_delay_milliseconds,
            DEFAULT_HTTP_RETRY_BASE_DELAY_MILLISECONDS
        );
        assert_eq!(
            retry_max_delay_milliseconds,
            DEFAULT_HTTP_RETRY_MAX_DELAY_MILLISECONDS
        );
    }
}
