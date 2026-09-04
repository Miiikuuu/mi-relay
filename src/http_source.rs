use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::{self, Read};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail};
use reqwest::StatusCode;
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{
    ACCEPT, ACCEPT_ENCODING, AUTHORIZATION, CACHE_CONTROL, CONTENT_ENCODING, CONTENT_LENGTH,
    CONTENT_TYPE, ETAG, HeaderValue, IF_MATCH, RETRY_AFTER,
};

use crate::model::{Delivery, MANIFEST_SCHEMA_VERSION};
use crate::protocol::{
    AcknowledgeRequest, DeliveryDescriptor, DeliveryIndex, PROTOCOL_HEADER, PROTOCOL_VERSION,
    ProblemDetails,
};
use crate::source::{DeliverySource, SourceIssue, SourceScan};
use crate::storage::{validate_delivery_id, validate_delivery_metadata};

const MAX_INDEX_BODY_BYTES: u64 = 4 * 1024 * 1024;
const MAX_ERROR_BODY_BYTES: u64 = 8 * 1024;
const MAX_CURSOR_BYTES: usize = 2048;
const MAX_PAGES_PER_SCAN: usize = 100;
const MAX_DELIVERIES_PER_SCAN: usize = 10_000;
const MAX_TOKEN_BYTES: usize = 4096;

pub struct HttpSource {
    client: Client,
    base_url: reqwest::Url,
    authorization: HeaderValue,
    page_size: u32,
    retry_policy: HttpRetryPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpRetryPolicy {
    max_attempts: u32,
    base_delay: Duration,
    max_delay: Duration,
}

#[derive(Debug)]
struct RetryableHttpError {
    message: String,
    retry_after: Option<Duration>,
}

struct HttpBodyReader {
    response: Response,
    expected_size: u64,
    bytes_read: u64,
    delivery_id: String,
}

impl HttpRetryPolicy {
    pub fn new(max_attempts: u32, base_delay: Duration, max_delay: Duration) -> Result<Self> {
        if max_attempts == 0 || max_attempts > crate::config::MAX_HTTP_RETRY_ATTEMPTS {
            bail!(
                "HTTP retry attempts must be between 1 and {}",
                crate::config::MAX_HTTP_RETRY_ATTEMPTS
            );
        }
        if max_delay < base_delay {
            bail!("HTTP retry maximum delay must not be shorter than its base delay");
        }
        if max_delay > Duration::from_millis(crate::config::MAX_HTTP_RETRY_DELAY_MILLISECONDS) {
            bail!(
                "HTTP retry maximum delay must not exceed {} milliseconds",
                crate::config::MAX_HTTP_RETRY_DELAY_MILLISECONDS
            );
        }
        Ok(Self {
            max_attempts,
            base_delay,
            max_delay,
        })
    }

    fn delay_after(&self, error: &anyhow::Error, failed_attempts: u32) -> Option<Duration> {
        if failed_attempts >= self.max_attempts {
            return None;
        }
        let retryable = retryable_http_error(error)?;
        if let Some(delay) = retryable.retry_after {
            return Some(delay.min(self.max_delay));
        }

        let exponent = failed_attempts.saturating_sub(1).min(31);
        let multiplier = 1_u32.checked_shl(exponent).unwrap_or(u32::MAX);
        let ceiling = self
            .base_delay
            .saturating_mul(multiplier)
            .min(self.max_delay);
        let ceiling_millis = u64::try_from(ceiling.as_millis()).unwrap_or(u64::MAX);
        if ceiling_millis <= 1 {
            return Some(ceiling);
        }
        let floor_millis = ceiling_millis / 2;
        Some(Duration::from_millis(fastrand::u64(
            floor_millis..=ceiling_millis,
        )))
    }
}

fn retryable_http_error(error: &anyhow::Error) -> Option<&RetryableHttpError> {
    error.chain().find_map(|cause| {
        cause.downcast_ref::<RetryableHttpError>().or_else(|| {
            cause
                .downcast_ref::<io::Error>()
                .and_then(io::Error::get_ref)
                .and_then(|inner| inner.downcast_ref::<RetryableHttpError>())
        })
    })
}

impl Default for HttpRetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: crate::config::DEFAULT_HTTP_RETRY_MAX_ATTEMPTS,
            base_delay: Duration::from_millis(
                crate::config::DEFAULT_HTTP_RETRY_BASE_DELAY_MILLISECONDS,
            ),
            max_delay: Duration::from_millis(
                crate::config::DEFAULT_HTTP_RETRY_MAX_DELAY_MILLISECONDS,
            ),
        }
    }
}

impl RetryableHttpError {
    fn new(message: String, retry_after: Option<Duration>) -> Self {
        Self {
            message,
            retry_after,
        }
    }
}

impl fmt::Display for RetryableHttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for RetryableHttpError {}

impl Read for HttpBodyReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.response.read(buffer).map_err(|error| {
            let kind = error.kind();
            io::Error::new(
                kind,
                RetryableHttpError::new(
                    format!(
                        "HTTP response body for delivery {} failed after {} bytes: {}",
                        self.delivery_id, self.bytes_read, error
                    ),
                    None,
                ),
            )
        })?;
        if read == 0 && self.bytes_read < self.expected_size {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                RetryableHttpError::new(
                    format!(
                        "HTTP response body for delivery {} ended after {} bytes; expected {}",
                        self.delivery_id, self.bytes_read, self.expected_size
                    ),
                    None,
                ),
            ));
        }
        self.bytes_read = self
            .bytes_read
            .checked_add(read as u64)
            .ok_or_else(|| io::Error::other("HTTP response byte count overflow"))?;
        Ok(read)
    }
}

type IndexPage = DeliveryIndex<serde_json::Value>;

impl HttpSource {
    pub fn new(
        base_url: &str,
        token: &str,
        request_timeout_seconds: u64,
        page_size: u32,
        allow_insecure_http: bool,
    ) -> Result<Self> {
        Self::new_with_retry(
            base_url,
            token,
            request_timeout_seconds,
            page_size,
            allow_insecure_http,
            HttpRetryPolicy::default(),
        )
    }

    pub fn new_with_retry(
        base_url: &str,
        token: &str,
        request_timeout_seconds: u64,
        page_size: u32,
        allow_insecure_http: bool,
        retry_policy: HttpRetryPolicy,
    ) -> Result<Self> {
        if request_timeout_seconds == 0 || request_timeout_seconds > 60 * 60 {
            bail!("HTTP request timeout must be between 1 and 3600 seconds");
        }
        if page_size == 0 || page_size > crate::config::MAX_HTTP_PAGE_SIZE {
            bail!(
                "HTTP page size must be between 1 and {}",
                crate::config::MAX_HTTP_PAGE_SIZE
            );
        }
        if token.is_empty() {
            bail!("HTTP bearer token must not be empty");
        }
        if token.len() > MAX_TOKEN_BYTES {
            bail!("HTTP bearer token exceeds the safety limit");
        }
        if token.chars().any(char::is_control) {
            bail!("HTTP bearer token contains control characters");
        }

        let mut base_url = reqwest::Url::parse(base_url).context("invalid HTTP server base URL")?;
        if base_url.host().is_none() {
            bail!("HTTP server base URL must include a host");
        }
        match base_url.scheme() {
            "https" => {}
            "http" if allow_insecure_http => {}
            "http" => bail!("refusing plain HTTP without allow_insecure_http"),
            scheme => bail!("unsupported HTTP server URL scheme {scheme:?}"),
        }
        if !base_url.username().is_empty() || base_url.password().is_some() {
            bail!("HTTP server base URL must not contain credentials");
        }
        if base_url.query().is_some() || base_url.fragment().is_some() {
            bail!("HTTP server base URL must not contain a query string or fragment");
        }
        if !base_url.path().ends_with('/') {
            let mut path = base_url.path().to_owned();
            path.push('/');
            base_url.set_path(&path);
        }

        let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))
            .context("HTTP bearer token cannot be represented as a header")?;
        authorization.set_sensitive(true);
        let connect_timeout = request_timeout_seconds.min(10);
        let client = Client::builder()
            .timeout(Duration::from_secs(request_timeout_seconds))
            .connect_timeout(Duration::from_secs(connect_timeout))
            .redirect(reqwest::redirect::Policy::none())
            .https_only(!allow_insecure_http)
            .user_agent(concat!("mirelay/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("failed to build HTTP client")?;

        Ok(Self {
            client,
            base_url,
            authorization,
            page_size,
            retry_policy,
        })
    }

    fn with_retry<T>(&self, operation: &str, mut attempt: impl FnMut() -> Result<T>) -> Result<T> {
        let mut failed_attempts = 0_u32;
        loop {
            match attempt() {
                Ok(value) => return Ok(value),
                Err(error) => {
                    failed_attempts = failed_attempts.saturating_add(1);
                    let Some(delay) = self.retry_policy.delay_after(&error, failed_attempts) else {
                        return if failed_attempts > 1 {
                            Err(error).with_context(|| {
                                format!("{operation} failed after {failed_attempts} attempts")
                            })
                        } else {
                            Err(error)
                        };
                    };
                    if !delay.is_zero() {
                        std::thread::sleep(delay);
                    }
                }
            }
        }
    }

    fn endpoint(&self, suffix: &[&str]) -> Result<reqwest::Url> {
        let mut url = self.base_url.clone();
        {
            let mut segments = url.path_segments_mut().map_err(|()| {
                anyhow::anyhow!("HTTP server base URL cannot contain path segments")
            })?;
            segments.pop_if_empty();
            segments.extend(["api", "v1", "deliveries"]);
            segments.extend(suffix.iter().copied());
        }
        Ok(url)
    }

    fn request(&self, builder: RequestBuilder) -> RequestBuilder {
        builder
            .header(AUTHORIZATION, self.authorization.clone())
            .header(PROTOCOL_HEADER, PROTOCOL_VERSION.to_string())
            .header(CACHE_CONTROL, "no-store")
    }

    fn fetch_page(&self, cursor: Option<&str>) -> Result<IndexPage> {
        let mut url = self.endpoint(&[])?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("status", "pending");
            query.append_pair("limit", &self.page_size.to_string());
            if let Some(cursor) = cursor {
                query.append_pair("cursor", cursor);
            }
        }
        self.with_retry("listing pending deliveries", || {
            let response = send_request(
                self.request(self.client.get(url.clone()))
                    .header(ACCEPT, "application/json"),
                "requesting pending deliveries",
            )?;
            let response = success_response(response, "listing pending deliveries")?;
            require_exact_status(&response, StatusCode::OK, "listing pending deliveries")?;
            require_content_type(&response, "application/json", "delivery index")?;
            let body = read_body_limited(response, MAX_INDEX_BODY_BYTES, true)
                .context("failed to read delivery index")?;
            let page: IndexPage =
                serde_json::from_slice(&body).context("failed to parse delivery index JSON")?;
            if page.schema_version != PROTOCOL_VERSION {
                bail!(
                    "server uses unsupported index schema {}; expected {}",
                    page.schema_version,
                    PROTOCOL_VERSION
                );
            }
            if page.items.len() > self.page_size as usize {
                bail!(
                    "server returned {} deliveries after a limit of {}",
                    page.items.len(),
                    self.page_size
                );
            }
            Ok(page)
        })
    }
}

impl DeliverySource for HttpSource {
    fn scan_pending(&self, max_file_size: u64) -> Result<SourceScan> {
        let mut scan = SourceScan::default();
        let mut by_id = BTreeMap::<String, Delivery>::new();
        let mut conflicts = BTreeSet::<String>::new();
        let mut seen_cursors = BTreeSet::<String>::new();
        let mut cursor = None::<String>;

        for page_number in 1..=MAX_PAGES_PER_SCAN {
            let page = self.fetch_page(cursor.as_deref())?;
            for (index, value) in page.items.into_iter().enumerate() {
                let item_name = format!("HTTP page {page_number}, item {}", index + 1);
                let wire: DeliveryDescriptor = match serde_json::from_value(value) {
                    Ok(wire) => wire,
                    Err(error) => {
                        scan.issues.push(SourceIssue {
                            item: item_name,
                            message: format!("invalid delivery object: {error}"),
                        });
                        continue;
                    }
                };
                let delivery = Delivery {
                    schema_version: MANIFEST_SCHEMA_VERSION,
                    id: wire.delivery_id,
                    original_name: wire.original_name,
                    payload: String::new(),
                    size: wire.size_bytes,
                    sha256: wire.sha256,
                    media_type: wire.media_type,
                    created_at_unix: wire.created_at_unix,
                };
                if let Err(error) = validate_delivery_metadata(&delivery, max_file_size) {
                    scan.issues.push(SourceIssue {
                        item: item_name,
                        message: format!("{error:#}"),
                    });
                    continue;
                }

                if let Some(previous) = by_id.get(&delivery.id) {
                    conflicts.insert(delivery.id.clone());
                    scan.issues.push(SourceIssue {
                        item: item_name,
                        message: format!(
                            "duplicate delivery id {} has digests {} and {}",
                            delivery.id, previous.sha256, delivery.sha256
                        ),
                    });
                } else if !conflicts.contains(&delivery.id) {
                    by_id.insert(delivery.id.clone(), delivery);
                }
                if by_id.len() + conflicts.len() > MAX_DELIVERIES_PER_SCAN {
                    bail!(
                        "server returned more than {} deliveries in one scan",
                        MAX_DELIVERIES_PER_SCAN
                    );
                }
            }

            cursor = match page.next_cursor {
                None => break,
                Some(next) => {
                    if next.is_empty()
                        || next.len() > MAX_CURSOR_BYTES
                        || next.chars().any(char::is_control)
                    {
                        bail!("server returned an invalid pagination cursor");
                    }
                    if !seen_cursors.insert(next.clone()) {
                        bail!("server repeated a pagination cursor");
                    }
                    Some(next)
                }
            };

            if page_number == MAX_PAGES_PER_SCAN {
                bail!(
                    "server pagination exceeded the {} page safety limit",
                    MAX_PAGES_PER_SCAN
                );
            }
        }

        for id in conflicts {
            by_id.remove(&id);
        }
        scan.deliveries = by_id.into_values().collect();
        scan.deliveries.sort_by(|left, right| {
            left.created_at_unix
                .unwrap_or(0)
                .cmp(&right.created_at_unix.unwrap_or(0))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(scan)
    }

    fn open_payload(&self, delivery: &Delivery) -> Result<Box<dyn Read + Send>> {
        validate_delivery_metadata(delivery, delivery.size)?;
        let url = self.endpoint(&[&delivery.id, "content"])?;
        let expected_etag = format!("\"sha256:{}\"", delivery.sha256);
        let response = send_request(
            self.request(self.client.get(url))
                .header(ACCEPT, &delivery.media_type)
                .header(ACCEPT_ENCODING, "identity")
                .header(IF_MATCH, &expected_etag),
            &format!("requesting delivery {}", delivery.id),
        )?;
        let response =
            success_response(response, &format!("downloading delivery {}", delivery.id))?;
        require_exact_status(
            &response,
            StatusCode::OK,
            &format!("downloading delivery {}", delivery.id),
        )?;
        require_content_type(&response, &delivery.media_type, "delivery content")?;

        if let Some(encoding) = response.headers().get(CONTENT_ENCODING) {
            let encoding = encoding
                .to_str()
                .context("delivery Content-Encoding is not valid ASCII")?;
            if !encoding.eq_ignore_ascii_case("identity") {
                bail!("server encoded delivery content as {encoding:?}; expected identity");
            }
        }
        let etag = response
            .headers()
            .get(ETAG)
            .context("delivery response is missing ETag")?
            .to_str()
            .context("delivery ETag is not valid ASCII")?;
        if etag != expected_etag {
            bail!(
                "delivery ETag mismatch: expected {}, got {}",
                expected_etag,
                etag
            );
        }
        let length = response
            .headers()
            .get(CONTENT_LENGTH)
            .context("delivery response is missing Content-Length")?
            .to_str()
            .context("delivery Content-Length is not valid ASCII")?
            .parse::<u64>()
            .context("delivery Content-Length is not an integer")?;
        if length != delivery.size {
            bail!(
                "delivery Content-Length mismatch: expected {}, got {}",
                delivery.size,
                length
            );
        }
        Ok(Box::new(HttpBodyReader {
            response,
            expected_size: delivery.size,
            bytes_read: 0,
            delivery_id: delivery.id.clone(),
        }))
    }

    fn acknowledge(&self, id: &str, sha256: &str) -> Result<()> {
        validate_delivery_id(id)?;
        if sha256.len() != 64
            || sha256 != sha256.to_ascii_lowercase()
            || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            bail!("cannot acknowledge delivery {id}: invalid SHA-256 digest");
        }
        let url = self.endpoint(&[id, "ack"])?;
        self.with_retry(&format!("acknowledging delivery {id}"), || {
            let response = send_request(
                self.request(self.client.put(url.clone()))
                    .header(ACCEPT, "application/json")
                    .json(&AcknowledgeRequest {
                        sha256: sha256.to_owned(),
                    }),
                &format!("requesting ACK for delivery {id}"),
            )?;
            let response = success_response(response, &format!("acknowledging delivery {id}"))?;
            match response.status() {
                StatusCode::NO_CONTENT | StatusCode::OK => Ok(()),
                status => bail!("unexpected successful ACK status {status}; expected 200 or 204"),
            }
        })
    }

    fn payload_retry_delay(&self, error: &anyhow::Error, failed_attempts: u32) -> Option<Duration> {
        self.retry_policy.delay_after(error, failed_attempts)
    }
}

fn send_request(builder: RequestBuilder, operation: &str) -> Result<Response> {
    builder.send().map_err(|error| {
        let message = format!("failed while {operation}: {error}");
        if error.is_builder() || error.is_redirect() || error.is_decode() {
            anyhow::anyhow!(message)
        } else {
            anyhow::Error::new(RetryableHttpError::new(message, None))
        }
    })
}

fn success_response(mut response: Response, operation: &str) -> Result<Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }

    let retry_after = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let retry_delay = retry_after.as_deref().and_then(parse_retry_after);
    let body = read_body_limited(&mut response, MAX_ERROR_BODY_BYTES, false).unwrap_or_default();
    let problem = serde_json::from_slice::<ProblemDetails>(&body).ok();
    let mut detail = format!("HTTP {status} while {operation}");
    if let Some(problem) = problem {
        if let Some(code) = problem.code {
            detail.push_str(&format!(" [{code}]"));
        }
        if let Some(message) = problem.detail {
            detail.push_str(&format!(": {message}"));
        }
        if let Some(request_id) = problem.request_id {
            detail.push_str(&format!(" (request_id={request_id})"));
        }
    }
    if let Some(retry_after) = retry_after {
        detail.push_str(&format!("; retry_after={retry_after}"));
    }
    if is_retryable_status(status) {
        Err(anyhow::Error::new(RetryableHttpError::new(
            detail,
            retry_delay,
        )))
    } else {
        bail!(detail)
    }
}

fn is_retryable_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT
            | StatusCode::TOO_EARLY
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

fn parse_retry_after(value: &str) -> Option<Duration> {
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let deadline = httpdate::parse_http_date(value).ok()?;
    Some(
        deadline
            .duration_since(SystemTime::now())
            .unwrap_or(Duration::ZERO),
    )
}

fn require_exact_status(response: &Response, expected: StatusCode, operation: &str) -> Result<()> {
    if response.status() != expected {
        bail!(
            "unexpected successful status {} while {}; expected {}",
            response.status(),
            operation,
            expected
        );
    }
    Ok(())
}

fn require_content_type(response: &Response, expected: &str, label: &str) -> Result<()> {
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .with_context(|| format!("{label} response is missing Content-Type"))?
        .to_str()
        .with_context(|| format!("{label} Content-Type is not valid ASCII"))?;
    let media_type = content_type.split(';').next().unwrap_or_default().trim();
    if !media_type.eq_ignore_ascii_case(expected) {
        bail!("{label} Content-Type is {media_type:?}; expected {expected:?}");
    }
    Ok(())
}

fn read_body_limited(
    mut response: impl Read,
    max_bytes: u64,
    reject_oversized: bool,
) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    response
        .by_ref()
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut body)
        .context("failed to read HTTP response body")?;
    if body.len() as u64 > max_bytes {
        if reject_oversized {
            bail!("HTTP response body exceeds the {max_bytes} byte safety limit");
        }
        body.truncate(max_bytes as usize);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_policy_recognizes_nested_transient_body_errors() {
        let body_error = io::Error::new(
            io::ErrorKind::UnexpectedEof,
            RetryableHttpError::new("truncated response".into(), None),
        );
        let error = anyhow::Error::new(body_error).context("failed to copy response");
        let policy = HttpRetryPolicy::new(2, Duration::ZERO, Duration::ZERO).unwrap();

        assert_eq!(policy.delay_after(&error, 1), Some(Duration::ZERO));
        assert_eq!(policy.delay_after(&error, 2), None);
        assert_eq!(
            policy.delay_after(&anyhow::anyhow!("invalid ETag"), 1),
            None
        );
    }

    #[test]
    fn retry_after_supports_seconds_and_http_dates() {
        assert_eq!(parse_retry_after("12"), Some(Duration::from_secs(12)));
        assert!(parse_retry_after("not-a-delay").is_none());
        let past = httpdate::fmt_http_date(SystemTime::UNIX_EPOCH);
        assert_eq!(parse_retry_after(&past), Some(Duration::ZERO));
    }
}
