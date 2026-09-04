use std::sync::Arc;

use anyhow::Result;
use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{DefaultBodyLimit, Path, Query, Request, State};
use axum::http::header::{
    AUTHORIZATION, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, ETAG, IF_MATCH, WWW_AUTHENTICATE,
};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio_util::io::ReaderStream;

use crate::protocol::{
    AcknowledgeRequest, DeliveryDescriptor, DeliveryIndex, PROTOCOL_HEADER, PROTOCOL_VERSION,
    ProblemDetails,
};

use super::store::{
    AcknowledgeOutcome, PendingPage, ServerStore, StoredDelivery, validate_device_id,
    validate_sha256,
};

const PROTOCOL_HEADER_NAME: HeaderName = HeaderName::from_static(PROTOCOL_HEADER);
const MAX_ACK_BODY_BYTES: usize = 16 * 1024;
const MAX_CURSOR_BYTES: usize = 2048;
const MAX_TOKEN_BYTES: usize = 4096;

#[derive(Clone)]
pub struct ApiState {
    pub(super) inner: Arc<ApiStateInner>,
}

pub(super) struct ApiStateInner {
    pub(super) store: ServerStore,
    pub(super) device_id: String,
    pub(super) authorization_hash: [u8; 32],
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    status: String,
    #[serde(default = "default_page_size")]
    limit: u32,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CursorV1 {
    version: u32,
    after_sequence: i64,
    snapshot_sequence: i64,
}

type IndexResponse = DeliveryIndex<DeliveryDescriptor>;

pub(super) struct ApiError {
    status: StatusCode,
    code: &'static str,
    detail: &'static str,
    request_id: String,
}

impl ApiState {
    pub fn new(store: ServerStore, device_id: String, token: &str) -> Result<Self> {
        validate_device_id(&device_id)?;
        validate_bearer_token(token)?;
        let mut hasher = Sha256::new();
        hasher.update(b"Bearer ");
        hasher.update(token.as_bytes());
        Ok(Self {
            inner: Arc::new(ApiStateInner {
                store,
                device_id,
                authorization_hash: hasher.finalize().into(),
            }),
        })
    }
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/api/v1/deliveries", get(list_deliveries))
        .route(
            "/api/v1/deliveries/{delivery_id}/content",
            get(download_content),
        )
        .route(
            "/api/v1/deliveries/{delivery_id}/ack",
            put(acknowledge_delivery),
        )
        .merge(super::tus_api::router())
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .layer(DefaultBodyLimit::max(MAX_ACK_BODY_BYTES))
        .layer(middleware::from_fn(add_security_headers))
        .with_state(state)
}

async fn health(State(state): State<ApiState>) -> Result<Response, ApiError> {
    let store = state.inner.store.clone();
    run_store("health check", move || store.health_check()).await?;
    Ok((StatusCode::OK, "ok\n").into_response())
}

async fn list_deliveries(
    State(state): State<ApiState>,
    headers: HeaderMap,
    query: Result<Query<ListQuery>, QueryRejection>,
) -> Result<Json<IndexResponse>, ApiError> {
    authorize(&state, &headers)?;
    let Query(query) = query.map_err(|_| {
        ApiError::bad_request("invalid_query", "invalid delivery-list query parameters")
    })?;
    if query.status != "pending" {
        return Err(ApiError::bad_request(
            "invalid_status",
            "status must be pending",
        ));
    }
    if query.limit == 0 || query.limit > 100 {
        return Err(ApiError::bad_request(
            "invalid_limit",
            "limit must be between 1 and 100",
        ));
    }
    let (after_sequence, snapshot_sequence) = match query.cursor {
        Some(cursor) => {
            let cursor = decode_cursor(&cursor)?;
            (cursor.after_sequence, Some(cursor.snapshot_sequence))
        }
        None => (0, None),
    };

    let store = state.inner.store.clone();
    let device_id = state.inner.device_id.clone();
    let limit = query.limit;
    let page = run_store("list pending deliveries", move || {
        store.list_pending(&device_id, after_sequence, snapshot_sequence, limit)
    })
    .await?;
    Ok(Json(index_response(page)?))
}

async fn download_content(
    State(state): State<ApiState>,
    Path(delivery_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    authorize(&state, &headers)?;
    crate::storage::validate_delivery_id(&delivery_id)
        .map_err(|_| ApiError::bad_request("invalid_delivery_id", "delivery id is invalid"))?;
    if headers.get_all(IF_MATCH).iter().count() != 1 {
        return Err(ApiError::new(
            StatusCode::PRECONDITION_REQUIRED,
            "if_match_required",
            "exactly one If-Match header is required",
        ));
    }

    let store = state.inner.store.clone();
    let device_id = state.inner.device_id.clone();
    let queried_id = delivery_id.clone();
    let delivery = run_store("query delivery content", move || {
        store.get_delivery(&device_id, &queried_id)
    })
    .await?
    .ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "delivery_not_found",
            "delivery was not found",
        )
    })?;
    let expected_etag = format!("\"sha256:{}\"", delivery.sha256);
    let supplied_etag = headers
        .get(IF_MATCH)
        .expect("header count was checked above")
        .as_bytes();
    if supplied_etag != expected_etag.as_bytes() {
        return Err(ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "etag_mismatch",
            "If-Match does not match the staged delivery",
        ));
    }

    let store = state.inner.store.clone();
    let opened_delivery = delivery.clone();
    let file = run_store("open delivery content", move || {
        store.open_content(&opened_delivery)
    })
    .await?;
    let file = tokio::fs::File::from_std(file);
    let stream = ReaderStream::new(file);
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, &delivery.media_type)
        .header(CONTENT_LENGTH, delivery.size)
        .header(ETAG, expected_etag)
        .body(Body::from_stream(stream))
        .map_err(|error| ApiError::internal("build content response", error.into()))
}

async fn acknowledge_delivery(
    State(state): State<ApiState>,
    Path(delivery_id): Path<String>,
    headers: HeaderMap,
    body: Result<Json<AcknowledgeRequest>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    authorize(&state, &headers)?;
    crate::storage::validate_delivery_id(&delivery_id)
        .map_err(|_| ApiError::bad_request("invalid_delivery_id", "delivery id is invalid"))?;
    let Json(body) =
        body.map_err(|_| ApiError::bad_request("invalid_json", "invalid ACK request body"))?;
    validate_sha256(&body.sha256)
        .map_err(|_| ApiError::bad_request("invalid_sha256", "sha256 is invalid"))?;

    let store = state.inner.store.clone();
    let device_id = state.inner.device_id.clone();
    let acknowledged_id = delivery_id.clone();
    let acknowledged_sha256 = body.sha256.clone();
    let outcome = run_store("acknowledge delivery", move || {
        store.acknowledge(&device_id, &acknowledged_id, &acknowledged_sha256)
    })
    .await?;
    match outcome {
        AcknowledgeOutcome::NotFound => {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                "delivery_not_found",
                "delivery was not found",
            ));
        }
        AcknowledgeOutcome::DigestConflict => {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "digest_conflict",
                "sha256 does not match the delivery",
            ));
        }
        AcknowledgeOutcome::Acknowledged | AcknowledgeOutcome::AlreadyAcknowledged => {}
    }

    let store = state.inner.store.clone();
    let collected_sha256 = body.sha256;
    if let Err(error) = run_store("collect acknowledged content", move || {
        store.garbage_collect_digest(&collected_sha256)
    })
    .await
    {
        log_nonfatal(&error);
    }
    Ok(StatusCode::NO_CONTENT)
}

fn index_response(page: PendingPage) -> Result<IndexResponse, ApiError> {
    let next_cursor = if page.has_more {
        let last = page.deliveries.last().ok_or_else(|| {
            ApiError::internal(
                "build pagination cursor",
                anyhow::anyhow!("store reported another page without any deliveries"),
            )
        })?;
        Some(encode_cursor(&CursorV1 {
            version: PROTOCOL_VERSION,
            after_sequence: last.sequence,
            snapshot_sequence: page.snapshot_sequence,
        })?)
    } else {
        None
    };
    Ok(IndexResponse {
        schema_version: PROTOCOL_VERSION,
        items: page
            .deliveries
            .into_iter()
            .map(DeliveryDescriptor::from)
            .collect(),
        next_cursor,
    })
}

impl From<StoredDelivery> for DeliveryDescriptor {
    fn from(delivery: StoredDelivery) -> Self {
        Self {
            delivery_id: delivery.id,
            original_name: delivery.original_name,
            size_bytes: delivery.size,
            sha256: delivery.sha256,
            media_type: delivery.media_type,
            created_at_unix: Some(delivery.created_at_unix),
        }
    }
}

fn encode_cursor(cursor: &CursorV1) -> Result<String, ApiError> {
    let bytes = serde_json::to_vec(cursor).map_err(|error| {
        ApiError::internal("serialize pagination cursor", anyhow::Error::from(error))
    })?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn decode_cursor(encoded: &str) -> Result<CursorV1, ApiError> {
    if encoded.is_empty()
        || encoded.len() > MAX_CURSOR_BYTES
        || encoded.chars().any(char::is_control)
    {
        return Err(ApiError::bad_request(
            "invalid_cursor",
            "pagination cursor is invalid",
        ));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| ApiError::bad_request("invalid_cursor", "pagination cursor is invalid"))?;
    let cursor: CursorV1 = serde_json::from_slice(&bytes)
        .map_err(|_| ApiError::bad_request("invalid_cursor", "pagination cursor is invalid"))?;
    if cursor.version != PROTOCOL_VERSION
        || cursor.after_sequence < 0
        || cursor.snapshot_sequence < cursor.after_sequence
    {
        return Err(ApiError::bad_request(
            "invalid_cursor",
            "pagination cursor is invalid",
        ));
    }
    Ok(cursor)
}

fn authorize(state: &ApiState, headers: &HeaderMap) -> Result<(), ApiError> {
    authorize_bearer(state, headers)?;

    if headers.get_all(&PROTOCOL_HEADER_NAME).iter().count() != 1
        || headers
            .get(&PROTOCOL_HEADER_NAME)
            .is_none_or(|value| value.as_bytes() != b"1")
    {
        return Err(ApiError::new(
            StatusCode::UPGRADE_REQUIRED,
            "unsupported_protocol",
            "mirelay-protocol-version must be 1",
        ));
    }
    Ok(())
}

pub(super) fn authorize_bearer(state: &ApiState, headers: &HeaderMap) -> Result<(), ApiError> {
    if headers.get_all(AUTHORIZATION).iter().count() != 1 {
        return Err(ApiError::unauthorized());
    }
    let authorization = headers
        .get(AUTHORIZATION)
        .expect("header count was checked above");
    let supplied_hash: [u8; 32] = Sha256::digest(authorization.as_bytes()).into();
    if state
        .inner
        .authorization_hash
        .ct_eq(&supplied_hash)
        .unwrap_u8()
        != 1
    {
        return Err(ApiError::unauthorized());
    }

    Ok(())
}

fn validate_bearer_token(token: &str) -> Result<()> {
    if token.is_empty() || token.len() > MAX_TOKEN_BYTES {
        anyhow::bail!("server bearer token must contain 1-{MAX_TOKEN_BYTES} bytes");
    }
    let mut saw_data = false;
    let mut saw_padding = false;
    for byte in token.bytes() {
        if byte == b'=' {
            saw_padding = true;
            continue;
        }
        let allowed =
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/');
        if !allowed || saw_padding {
            anyhow::bail!("server bearer token is not a valid RFC 6750 token");
        }
        saw_data = true;
    }
    if !saw_data {
        anyhow::bail!("server bearer token must contain token data");
    }
    Ok(())
}

pub(super) async fn run_store<T, F>(operation: &'static str, task: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    match tokio::task::spawn_blocking(task).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(ApiError::internal(operation, error)),
        Err(error) => Err(ApiError::internal(operation, error.into())),
    }
}

async fn add_security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(
        HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("no-referrer"),
    );
    response
}

async fn not_found() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "route_not_found",
        "route was not found",
    )
}

async fn method_not_allowed() -> ApiError {
    ApiError::new(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "HTTP method is not allowed for this route",
    )
}

impl ApiError {
    pub(super) fn new(status: StatusCode, code: &'static str, detail: &'static str) -> Self {
        Self {
            status,
            code,
            detail,
            request_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    pub(super) fn bad_request(code: &'static str, detail: &'static str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, detail)
    }

    pub(super) fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "valid bearer authentication is required",
        )
    }

    pub(super) fn internal(operation: &'static str, error: anyhow::Error) -> Self {
        let request_id = uuid::Uuid::new_v4().to_string();
        eprintln!(
            "server error request_id={}: {}: {}",
            request_id,
            operation,
            crate::cli::terminal_safe(&format!("{error:#}"))
        );
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error",
            detail: "the server could not complete the request",
            request_id,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let title = self
            .status
            .canonical_reason()
            .unwrap_or("MiRelay protocol error");
        let problem = ProblemDetails {
            type_url: Some("about:blank".to_owned()),
            title: Some(title.to_owned()),
            status: Some(self.status.as_u16()),
            code: Some(self.code.to_owned()),
            detail: Some(self.detail.to_owned()),
            request_id: Some(self.request_id.clone()),
        };
        let mut response = (self.status, Json(problem)).into_response();
        response.headers_mut().insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/problem+json"),
        );
        if self.status == StatusCode::UNAUTHORIZED {
            response
                .headers_mut()
                .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        if self.status == StatusCode::UPGRADE_REQUIRED {
            response
                .headers_mut()
                .insert(PROTOCOL_HEADER_NAME, HeaderValue::from_static("1"));
        }
        response
    }
}

fn log_nonfatal(error: &ApiError) {
    eprintln!(
        "server non-fatal error request_id={}: {}",
        error.request_id, error.detail
    );
}

fn default_page_size() -> u32 {
    50
}

#[cfg(test)]
mod tests {
    use std::fs;

    use axum::body::to_bytes;
    use axum::http::Request;
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;

    const TOKEN: &str = "server-test-token";

    fn png_bytes() -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(b"MiRelay server API test");
        bytes
    }

    fn authenticated_request(method: &str, uri: &str) -> axum::http::request::Builder {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header(PROTOCOL_HEADER, "1")
    }

    #[tokio::test]
    async fn api_lists_streams_and_idempotently_acknowledges() {
        let root = tempfile::tempdir().unwrap();
        let image = root.path().join("image.png");
        let bytes = png_bytes();
        fs::write(&image, &bytes).unwrap();
        let store = ServerStore::new(root.path().join("server"), 1024).unwrap();
        store.initialize().unwrap();
        let delivery = store
            .enqueue("linux", &image, "illustration.png".into())
            .unwrap();
        let state = ApiState::new(store.clone(), "linux".into(), TOKEN).unwrap();
        let app = router(state);

        let response = app
            .clone()
            .oneshot(
                authenticated_request("GET", "/api/v1/deliveries?status=pending&limit=50")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CACHE_CONTROL], "no-store");
        let index: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(index["schema_version"], 1);
        assert_eq!(index["items"][0]["delivery_id"], delivery.id);
        assert_eq!(index["items"][0]["sha256"], delivery.sha256);

        let etag = format!("\"sha256:{}\"", delivery.sha256);
        let response = app
            .clone()
            .oneshot(
                authenticated_request(
                    "GET",
                    &format!("/api/v1/deliveries/{}/content", delivery.id),
                )
                .header(IF_MATCH, &etag)
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[ETAG], etag);
        assert_eq!(
            to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
            bytes
        );

        let ack_uri = format!("/api/v1/deliveries/{}/ack", delivery.id);
        let ack_body = serde_json::json!({ "sha256": delivery.sha256 }).to_string();
        for _ in 0..2 {
            let response = app
                .clone()
                .oneshot(
                    authenticated_request("PUT", &ack_uri)
                        .header(CONTENT_TYPE, "application/json")
                        .body(Body::from(ack_body.clone()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
        }
        assert!(store.open_content(&delivery).is_err());

        let response = app
            .oneshot(
                authenticated_request("GET", "/api/v1/deliveries?status=pending&limit=50")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let index: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(index["items"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn api_rejects_unauthorized_and_conflicting_ack_requests() {
        let root = tempfile::tempdir().unwrap();
        let image = root.path().join("image.png");
        fs::write(&image, png_bytes()).unwrap();
        let store = ServerStore::new(root.path().join("server"), 1024).unwrap();
        store.initialize().unwrap();
        let delivery = store
            .enqueue("linux", &image, "illustration.png".into())
            .unwrap();
        let app = router(ApiState::new(store, "linux".into(), TOKEN).unwrap());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/deliveries?status=pending&limit=50")
                    .header(AUTHORIZATION, "Bearer wrong-token")
                    .header(PROTOCOL_HEADER, "1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()[WWW_AUTHENTICATE], "Bearer");

        let response = app
            .oneshot(
                authenticated_request("PUT", &format!("/api/v1/deliveries/{}/ack", delivery.id))
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({ "sha256": "0".repeat(64) }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let problem: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(problem["code"], "digest_conflict");
    }
}
