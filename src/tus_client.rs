use std::env;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use clap::Parser;
use reqwest::StatusCode;
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, LOCATION};
use serde::{Deserialize, Serialize};

use crate::fsutil::{atomic_write, read_limited, sync_directory};
use crate::protocol::ProblemDetails;
use crate::storage::{FileInspection, inspect_file};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

const TUS_VERSION: &str = "1.0.0";
const TUS_RESUMABLE: &str = "tus-resumable";
const TUS_VERSIONS: &str = "tus-version";
const TUS_EXTENSION: &str = "tus-extension";
const TUS_MAX_SIZE: &str = "tus-max-size";
const UPLOAD_LENGTH: &str = "upload-length";
const UPLOAD_OFFSET: &str = "upload-offset";
const UPLOAD_METADATA: &str = "upload-metadata";
const MIRELAY_DELIVERY_ID: &str = "mirelay-delivery-id";
const OFFSET_CONTENT_TYPE: &str = "application/offset+octet-stream";
const STATE_SCHEMA_VERSION: u32 = 1;
const MAX_STATE_BYTES: u64 = 64 * 1024;
const MAX_ERROR_BODY_BYTES: u64 = 8 * 1024;
const MAX_TOKEN_BYTES: usize = 4096;
const DEFAULT_CHUNK_SIZE: u64 = 1024 * 1024;
const MAX_CHUNK_SIZE: u64 = 16 * 1024 * 1024;

#[derive(Debug, Parser)]
#[command(
    name = "mirelay-upload",
    version,
    about = "Resumable tus test sender for MiRelay"
)]
struct UploadCli {
    /// File to upload.
    #[arg(value_name = "FILE")]
    file: PathBuf,

    /// Public base URL of the MiRelay server.
    #[arg(long, value_name = "URL")]
    server_url: String,

    /// Durable resume state. Removed after confirmed completion.
    #[arg(long, value_name = "PATH")]
    state_file: PathBuf,

    /// Read the bearer token from this environment variable.
    #[arg(long, default_value = "MIRELAY_TOKEN", value_name = "NAME")]
    token_env: String,

    /// Name exposed to the receiving Linux device.
    #[arg(long, value_name = "NAME")]
    name: Option<String>,

    /// Bytes sent in each tus PATCH.
    #[arg(
        long,
        default_value_t = DEFAULT_CHUNK_SIZE,
        value_parser = clap::value_parser!(u64).range(1..=MAX_CHUNK_SIZE),
        value_name = "BYTES"
    )]
    chunk_size_bytes: u64,

    /// Stop successfully after this many PATCH requests, leaving resume state.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..), value_name = "COUNT")]
    max_chunks: Option<u64>,

    /// Allow plain HTTP for an isolated test network.
    #[arg(long)]
    allow_insecure_http: bool,

    /// HTTP request timeout.
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u64).range(1..=3600), value_name = "SECONDS")]
    request_timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UploadState {
    schema_version: u32,
    collection_url: String,
    upload_url: String,
    original_name: String,
    size: u64,
    sha256: String,
    media_type: String,
}

struct TusClient {
    client: Client,
    collection_url: reqwest::Url,
    authorization: HeaderValue,
}

#[derive(Debug)]
struct RemoteUpload {
    offset: u64,
    length: u64,
    metadata: String,
    delivery_id: Option<String>,
}

#[derive(Debug)]
struct HttpStatusError {
    status: StatusCode,
    message: String,
}

impl fmt::Display for HttpStatusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for HttpStatusError {}

pub fn run() -> Result<()> {
    run_cli(UploadCli::parse())
}

fn run_cli(cli: UploadCli) -> Result<()> {
    validate_environment_variable_name(&cli.token_env)?;
    let token = env::var(&cli.token_env).with_context(|| {
        format!(
            "upload token environment variable {} is not set or is not valid UTF-8",
            cli.token_env
        )
    })?;
    let client = TusClient::new(
        &cli.server_url,
        &token,
        cli.request_timeout_seconds,
        cli.allow_insecure_http,
    )?;
    drop(token);
    let file = absolute_path(cli.file)?;
    let state_file = absolute_path(cli.state_file)?;

    let (state, inspection) = if state_file.exists() {
        let state = load_state(&state_file)?;
        client.validate_state(&state)?;
        let inspection = inspect_file(&file, state.size)
            .context("the source file no longer matches resumable upload state")?;
        validate_source_state(&state, &inspection, cli.name.as_deref())?;
        println!("Resuming tus upload {}", safe_url(&state.upload_url));
        (state, inspection)
    } else {
        let server_max_size = client.discover()?;
        let inspection = inspect_file(&file, server_max_size)?;
        let original_name = match cli.name {
            Some(name) => name,
            None => file
                .file_name()
                .and_then(|name| name.to_str())
                .context("file name is not valid UTF-8; pass --name")?
                .to_owned(),
        };
        let metadata = metadata_header(&original_name, &inspection);
        let upload_url = client.create(&inspection, &metadata)?;
        let state = UploadState {
            schema_version: STATE_SCHEMA_VERSION,
            collection_url: client.collection_url.to_string(),
            upload_url: upload_url.to_string(),
            original_name,
            size: inspection.size,
            sha256: inspection.sha256.clone(),
            media_type: inspection.media_type.clone(),
        };
        save_state(&state_file, &state)?;
        println!("Created tus upload {}", safe_url(&state.upload_url));
        (state, inspection)
    };

    let upload_url = client.validate_upload_url(&state.upload_url)?;
    let expected_metadata = metadata_header(&state.original_name, &inspection);
    let mut remote = match client.head(&upload_url) {
        Ok(remote) => remote,
        Err(error) if should_discard_upload_state(&error) => {
            remove_state(&state_file)?;
            return Err(error).context(format!(
                "server rejected or removed the upload; local resume state {} was discarded, so rerun the command to create a new upload",
                state_file.display()
            ));
        }
        Err(error) => return Err(error),
    };
    validate_remote_upload(&state, &expected_metadata, &remote)?;
    let mut chunks_sent = 0_u64;

    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_CLOEXEC);
    let mut source = options
        .open(&file)
        .with_context(|| format!("failed to open source file {}", file.display()))?;
    while remote.offset < state.size {
        let remaining = state.size - remote.offset;
        let chunk_length = remaining.min(cli.chunk_size_bytes) as usize;
        let mut chunk = vec![0_u8; chunk_length];
        source
            .seek(SeekFrom::Start(remote.offset))
            .with_context(|| format!("failed to seek source file {}", file.display()))?;
        source
            .read_exact(&mut chunk)
            .with_context(|| format!("failed to read source file {}", file.display()))?;

        remote = match client.patch(&upload_url, remote.offset, chunk) {
            Ok(upload) => upload,
            Err(error) if should_discard_upload_state(&error) => {
                remove_state(&state_file)?;
                return Err(error).context(format!(
                    "server rejected or removed the upload; local resume state {} was discarded, so rerun the command to create a new upload",
                    state_file.display()
                ));
            }
            Err(error) => {
                return Err(error).context(format!(
                    "tus PATCH failed; rerun with the same --state-file {} to resume",
                    state_file.display()
                ));
            }
        };
        validate_remote_upload(&state, &expected_metadata, &remote)?;
        chunks_sent += 1;
        println!("  uploaded: {}/{} bytes", remote.offset, state.size);
        if cli.max_chunks.is_some_and(|limit| chunks_sent >= limit) && remote.offset < state.size {
            println!("Paused after {chunks_sent} chunk(s).");
            println!("Resume state: {}", safe_path(&state_file));
            return Ok(());
        }
    }

    if remote.delivery_id.is_none() {
        remote = match client.head(&upload_url) {
            Ok(remote) => remote,
            Err(error) if should_discard_upload_state(&error) => {
                remove_state(&state_file)?;
                return Err(error).context(format!(
                    "server rejected or removed the upload; local resume state {} was discarded, so rerun the command to create a new upload",
                    state_file.display()
                ));
            }
            Err(error) => return Err(error),
        };
        validate_remote_upload(&state, &expected_metadata, &remote)?;
    }
    let delivery_id = remote
        .delivery_id
        .context("server reached the final tus offset without confirming a MiRelay delivery id")?;
    remove_state(&state_file)?;
    println!("Upload complete");
    println!("  delivery: {delivery_id}");
    println!("  sha256:   {}", state.sha256);
    println!("  size:     {}", state.size);
    Ok(())
}

impl TusClient {
    fn new(
        base_url: &str,
        token: &str,
        timeout_seconds: u64,
        allow_insecure_http: bool,
    ) -> Result<Self> {
        if token.is_empty() || token.len() > MAX_TOKEN_BYTES || token.chars().any(char::is_control)
        {
            bail!("HTTP bearer token is empty, too long, or contains control characters");
        }
        let mut base_url = reqwest::Url::parse(base_url).context("invalid server base URL")?;
        if base_url.host().is_none() {
            bail!("server base URL must include a host");
        }
        match base_url.scheme() {
            "https" => {}
            "http" if allow_insecure_http => {}
            "http" => bail!("refusing plain HTTP without --allow-insecure-http"),
            scheme => bail!("unsupported server URL scheme {scheme:?}"),
        }
        if !base_url.username().is_empty() || base_url.password().is_some() {
            bail!("server base URL must not contain credentials");
        }
        if base_url.query().is_some() || base_url.fragment().is_some() {
            bail!("server base URL must not contain a query string or fragment");
        }
        if !base_url.path().ends_with('/') {
            let mut path = base_url.path().to_owned();
            path.push('/');
            base_url.set_path(&path);
        }
        {
            let mut segments = base_url
                .path_segments_mut()
                .map_err(|()| anyhow::anyhow!("server base URL cannot contain path segments"))?;
            segments.pop_if_empty();
            segments.extend(["api", "v1", "uploads"]);
        }
        let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))
            .context("HTTP bearer token cannot be represented as a header")?;
        authorization.set_sensitive(true);
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_seconds))
            .connect_timeout(Duration::from_secs(timeout_seconds.min(10)))
            .redirect(reqwest::redirect::Policy::none())
            .https_only(!allow_insecure_http)
            .user_agent(concat!("mirelay-upload/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("failed to build tus HTTP client")?;
        Ok(Self {
            client,
            collection_url: base_url,
            authorization,
        })
    }

    fn discover(&self) -> Result<u64> {
        let response = self
            .client
            .request(reqwest::Method::OPTIONS, self.collection_url.clone())
            .send()
            .context("failed to discover tus server capabilities")?;
        let response = require_status(
            response,
            &[StatusCode::NO_CONTENT, StatusCode::OK],
            "OPTIONS",
        )?;
        let versions = required_header(response.headers(), TUS_VERSIONS)?;
        if !versions
            .split(',')
            .map(str::trim)
            .any(|version| version == TUS_VERSION)
        {
            bail!("server does not advertise tus {TUS_VERSION}");
        }
        let extensions = required_header(response.headers(), TUS_EXTENSION)?;
        if !extensions
            .split(',')
            .map(str::trim)
            .any(|extension| extension == "creation")
        {
            bail!("server does not advertise the tus creation extension");
        }
        parse_u64(
            required_header(response.headers(), TUS_MAX_SIZE)?,
            TUS_MAX_SIZE,
        )
    }

    fn create(&self, inspection: &FileInspection, metadata: &str) -> Result<reqwest::Url> {
        let response = self
            .request(self.client.post(self.collection_url.clone()))
            .header(UPLOAD_LENGTH, inspection.size)
            .header(UPLOAD_METADATA, metadata)
            .body(Vec::new())
            .send()
            .context("failed to create tus upload")?;
        let response = require_status(response, &[StatusCode::CREATED], "create upload")?;
        require_tus_response(&response)?;
        let location = required_header(response.headers(), LOCATION.as_str())?;
        let upload_url = self
            .collection_url
            .join(location)
            .context("server returned an invalid tus Location")?;
        self.validate_upload_url(upload_url.as_str())
    }

    fn head(&self, upload_url: &reqwest::Url) -> Result<RemoteUpload> {
        let response = self
            .request(self.client.head(upload_url.clone()))
            .send()
            .context("failed to query tus upload offset")?;
        let response = require_status(
            response,
            &[StatusCode::OK, StatusCode::NO_CONTENT],
            "HEAD upload",
        )?;
        require_tus_response(&response)?;
        remote_upload(response.headers())
    }

    fn patch(
        &self,
        upload_url: &reqwest::Url,
        offset: u64,
        chunk: Vec<u8>,
    ) -> Result<RemoteUpload> {
        let response = self
            .request(self.client.patch(upload_url.clone()))
            .header(CONTENT_TYPE, OFFSET_CONTENT_TYPE)
            .header(UPLOAD_OFFSET, offset)
            .body(chunk)
            .send()
            .context("failed to send tus PATCH")?;
        if response.status() == StatusCode::CONFLICT {
            require_tus_response(&response)?;
            let server_offset = parse_u64(
                required_header(response.headers(), UPLOAD_OFFSET)?,
                UPLOAD_OFFSET,
            )?;
            let recovered = self.head(upload_url)?;
            if recovered.offset != server_offset {
                bail!(
                    "server reported conflicting tus offsets {server_offset} and {}",
                    recovered.offset
                );
            }
            return Ok(recovered);
        }
        let response = require_status(response, &[StatusCode::NO_CONTENT], "PATCH upload")?;
        require_tus_response(&response)?;
        remote_upload(response.headers())
    }

    fn request(&self, builder: RequestBuilder) -> RequestBuilder {
        builder
            .header(AUTHORIZATION, self.authorization.clone())
            .header(TUS_RESUMABLE, TUS_VERSION)
            .header("cache-control", "no-store")
    }

    fn validate_state(&self, state: &UploadState) -> Result<()> {
        if state.schema_version != STATE_SCHEMA_VERSION {
            bail!(
                "unsupported upload state schema {}; expected {}",
                state.schema_version,
                STATE_SCHEMA_VERSION
            );
        }
        if state.collection_url != self.collection_url.as_str() {
            bail!("upload state belongs to a different server URL");
        }
        self.validate_upload_url(&state.upload_url)?;
        Ok(())
    }

    fn validate_upload_url(&self, value: &str) -> Result<reqwest::Url> {
        let url = reqwest::Url::parse(value).context("tus upload URL is invalid")?;
        if url.scheme() != self.collection_url.scheme()
            || url.host_str() != self.collection_url.host_str()
            || url.port_or_known_default() != self.collection_url.port_or_known_default()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            bail!("refusing a tus upload URL outside the configured server origin");
        }
        let prefix = format!("{}/", self.collection_url.path().trim_end_matches('/'));
        let Some(upload_id) = url.path().strip_prefix(&prefix) else {
            bail!("tus upload URL is outside the configured upload collection");
        };
        if upload_id.is_empty()
            || upload_id.contains('/')
            || uuid::Uuid::parse_str(upload_id)
                .ok()
                .is_none_or(|id| id.to_string() != upload_id)
        {
            bail!("tus upload URL does not contain a canonical upload id");
        }
        Ok(url)
    }
}

fn remote_upload(headers: &HeaderMap) -> Result<RemoteUpload> {
    Ok(RemoteUpload {
        offset: parse_u64(required_header(headers, UPLOAD_OFFSET)?, UPLOAD_OFFSET)?,
        length: parse_u64(required_header(headers, UPLOAD_LENGTH)?, UPLOAD_LENGTH)?,
        metadata: required_header(headers, UPLOAD_METADATA)?.to_owned(),
        delivery_id: optional_header(headers, MIRELAY_DELIVERY_ID)?.map(str::to_owned),
    })
}

fn validate_remote_upload(
    state: &UploadState,
    expected_metadata: &str,
    remote: &RemoteUpload,
) -> Result<()> {
    if remote.length != state.size {
        bail!(
            "server upload length is {}, expected {}",
            remote.length,
            state.size
        );
    }
    if remote.offset > remote.length {
        bail!("server upload offset exceeds its declared length");
    }
    if remote.metadata != expected_metadata {
        bail!("server upload metadata no longer matches the source file");
    }
    if remote.delivery_id.is_some() && remote.offset != remote.length {
        bail!("server confirmed a delivery before the tus upload completed");
    }
    Ok(())
}

fn validate_source_state(
    state: &UploadState,
    inspection: &FileInspection,
    requested_name: Option<&str>,
) -> Result<()> {
    if inspection.size != state.size
        || inspection.sha256 != state.sha256
        || inspection.media_type != state.media_type
    {
        bail!("source file content differs from the saved upload state");
    }
    if requested_name.is_some_and(|name| name != state.original_name) {
        bail!("--name differs from the name saved in upload state");
    }
    Ok(())
}

fn metadata_header(original_name: &str, inspection: &FileInspection) -> String {
    [
        ("filename", original_name),
        ("media_type", inspection.media_type.as_str()),
        ("sha256", inspection.sha256.as_str()),
    ]
    .into_iter()
    .map(|(key, value)| format!("{key} {}", STANDARD.encode(value)))
    .collect::<Vec<_>>()
    .join(",")
}

fn require_tus_response(response: &Response) -> Result<()> {
    if required_header(response.headers(), TUS_RESUMABLE)? != TUS_VERSION {
        bail!("server response does not confirm tus {TUS_VERSION}");
    }
    Ok(())
}

fn require_status(
    mut response: Response,
    expected: &[StatusCode],
    operation: &str,
) -> Result<Response> {
    if expected.contains(&response.status()) {
        return Ok(response);
    }
    let status = response.status();
    let mut body = Vec::new();
    response
        .by_ref()
        .take(MAX_ERROR_BODY_BYTES.saturating_add(1))
        .read_to_end(&mut body)
        .context("failed to read server error response")?;
    let detail = serde_json::from_slice::<ProblemDetails>(&body)
        .ok()
        .and_then(|problem| problem.detail)
        .unwrap_or_else(|| String::from_utf8_lossy(&body).trim().to_owned());
    let message = if detail.is_empty() {
        format!("{operation} returned HTTP {status}")
    } else {
        format!(
            "{operation} returned HTTP {status}: {}",
            crate::cli::terminal_safe(&detail)
        )
    };
    Err(HttpStatusError { status, message }.into())
}

fn should_discard_upload_state(error: &anyhow::Error) -> bool {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<HttpStatusError>())
        .is_some_and(|error| {
            matches!(
                error.status,
                StatusCode::NOT_FOUND | StatusCode::GONE | StatusCode::UNPROCESSABLE_ENTITY
            )
        })
}

fn required_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str> {
    if headers.get_all(name).iter().count() != 1 {
        bail!("server response must contain exactly one {name} header");
    }
    headers
        .get(name)
        .expect("header count was checked")
        .to_str()
        .with_context(|| format!("server {name} header is not valid ASCII"))
}

fn optional_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>> {
    let count = headers.get_all(name).iter().count();
    match count {
        0 => Ok(None),
        1 => Ok(Some(
            headers
                .get(name)
                .expect("header count was checked")
                .to_str()
                .with_context(|| format!("server {name} header is not valid ASCII"))?,
        )),
        _ => bail!("server response contains more than one {name} header"),
    }
}

fn parse_u64(value: &str, field: &str) -> Result<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("server {field} is not an unsigned decimal integer");
    }
    value
        .parse()
        .with_context(|| format!("server {field} is out of range"))
}

fn load_state(path: &Path) -> Result<UploadState> {
    let bytes = read_limited(path, MAX_STATE_BYTES)
        .with_context(|| format!("failed to read upload state {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse upload state {}", path.display()))
}

fn save_state(path: &Path, state: &UploadState) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(state).context("failed to serialize upload state")?;
    atomic_write(path, &bytes)
        .with_context(|| format!("failed to save upload state {}", path.display()))
}

fn remove_state(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => {
            if let Some(parent) = path.parent() {
                sync_directory(if parent.as_os_str().is_empty() {
                    Path::new(".")
                } else {
                    parent
                })?;
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("failed to remove upload state {}", path.display()))
        }
    }
}

fn validate_environment_variable_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 128
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        bail!("token environment variable name is invalid");
    }
    Ok(())
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

fn safe_url(value: &str) -> String {
    crate::cli::terminal_safe(value)
}

fn safe_path(path: &Path) -> String {
    crate::cli::terminal_safe(&path.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::*;

    #[test]
    fn upload_location_cannot_move_the_bearer_token_to_another_origin() {
        let client = TusClient::new("https://example.test/prefix", "secret", 30, false).unwrap();
        assert!(
            client
                .validate_upload_url(
                    "https://example.test/prefix/api/v1/uploads/00000000-0000-4000-8000-000000000001"
                )
                .is_ok()
        );
        assert!(
            client
                .validate_upload_url(
                    "https://attacker.test/prefix/api/v1/uploads/00000000-0000-4000-8000-000000000001"
                )
                .is_err()
        );
    }

    #[test]
    fn metadata_is_stable_across_resume() {
        let bytes = b"image";
        let inspection = FileInspection {
            size: bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(bytes)),
            media_type: "image/png".to_owned(),
            extension: "png",
        };
        assert_eq!(
            metadata_header("a.png", &inspection),
            metadata_header("a.png", &inspection)
        );
    }

    #[test]
    fn only_terminal_upload_statuses_discard_resume_state() {
        for status in [
            StatusCode::NOT_FOUND,
            StatusCode::GONE,
            StatusCode::UNPROCESSABLE_ENTITY,
        ] {
            let error = anyhow::Error::new(HttpStatusError {
                status,
                message: "terminal upload response".to_owned(),
            });
            assert!(should_discard_upload_state(&error));
        }

        let transient = anyhow::Error::new(HttpStatusError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "try again later".to_owned(),
        });
        assert!(!should_discard_upload_state(&transient));
    }
}
