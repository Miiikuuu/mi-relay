use std::collections::{BTreeMap, HashMap};
use std::io;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{OriginalUri, Path, Request, State};
use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE, LOCATION};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::options;
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use futures_util::TryStreamExt;
use tokio::io::AsyncReadExt;
use tokio_util::io::StreamReader;

use super::api::{ApiError, ApiState};
use super::folders::authorize_transfer;
use super::tus_store::{AppendUploadOutcome, InvalidUploadContent, NewUpload, UploadInfo};

const TUS_VERSION_VALUE: &str = "1.0.0";
const TUS_RESUMABLE: HeaderName = HeaderName::from_static("tus-resumable");
const TUS_VERSION: HeaderName = HeaderName::from_static("tus-version");
const TUS_EXTENSION: HeaderName = HeaderName::from_static("tus-extension");
const TUS_MAX_SIZE: HeaderName = HeaderName::from_static("tus-max-size");
const UPLOAD_LENGTH: HeaderName = HeaderName::from_static("upload-length");
const UPLOAD_DEFER_LENGTH: HeaderName = HeaderName::from_static("upload-defer-length");
const UPLOAD_OFFSET: HeaderName = HeaderName::from_static("upload-offset");
const UPLOAD_METADATA: HeaderName = HeaderName::from_static("upload-metadata");
const MIRELAY_DELIVERY_ID: HeaderName = HeaderName::from_static("mirelay-delivery-id");
const OFFSET_CONTENT_TYPE: &str = "application/offset+octet-stream";
const MAX_METADATA_HEADER_BYTES: usize = 8 * 1024;

pub(super) fn router() -> Router<ApiState> {
    Router::new()
        .route("/api/v1/uploads", options(tus_options).post(create_upload))
        .route(
            "/api/v1/uploads/{upload_id}",
            options(tus_options).head(head_upload).patch(patch_upload),
        )
}

async fn tus_options(State(state): State<ApiState>) -> Result<Response, TusError> {
    tus_response(StatusCode::NO_CONTENT, |builder| {
        builder
            .header(TUS_VERSION, TUS_VERSION_VALUE)
            .header(TUS_EXTENSION, "creation")
            .header(TUS_MAX_SIZE, state.inner.store.max_file_size())
    })
}

async fn create_upload(
    State(state): State<ApiState>,
    OriginalUri(uri): OriginalUri,
    request: Request,
) -> Result<Response, TusError> {
    let device_id = authorize_transfer(&state, request.headers(), &uri, "sender")
        .await
        .map_err(TusError::from)?;
    require_tus_version(request.headers())?;
    if request.headers().contains_key(&UPLOAD_DEFER_LENGTH) {
        return Err(TusError::bad_request(
            "upload_defer_unsupported",
            "deferred upload length is not supported",
        ));
    }
    let length = required_u64_header(request.headers(), &UPLOAD_LENGTH, "upload_length")?;
    if length > state.inner.store.max_file_size() {
        return Err(TusError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "upload_too_large",
            "upload length exceeds the server limit",
        ));
    }
    let metadata = parse_upload_metadata(request.headers())?;
    let body = to_bytes(request.into_body(), 1).await.map_err(|_| {
        TusError::bad_request(
            "creation_body_unsupported",
            "creation-with-upload is not supported",
        )
    })?;
    if !body.is_empty() {
        return Err(TusError::bad_request(
            "creation_body_unsupported",
            "creation-with-upload is not supported",
        ));
    }

    let upload = NewUpload {
        directory: match (
            metadata.get("relative_path"),
            metadata.get("source_version"),
        ) {
            (None, None) => None,
            (Some(path), Some(version)) => Some(crate::directory::DirectoryVersion {
                path: path.clone(),
                version: version.parse().map_err(|_| {
                    TusError::bad_request("invalid_directory_version", "Invalid directory version.")
                })?,
            }),
            _ => {
                return Err(TusError::bad_request(
                    "incomplete_directory_metadata",
                    "Directory path and version must be supplied together.",
                ));
            }
        },
        original_name: metadata
            .get("filename")
            .expect("required metadata was checked")
            .clone(),
        media_type: metadata
            .get("media_type")
            .expect("required metadata was checked")
            .clone(),
        sha256: metadata
            .get("sha256")
            .expect("required metadata was checked")
            .clone(),
        length,
        metadata_header: canonical_metadata_header(&metadata),
    };
    state
        .inner
        .store
        .validate_new_upload(&upload)
        .map_err(|_| {
            TusError::bad_request(
                "invalid_upload_metadata",
                "upload metadata is invalid or unsupported",
            )
        })?;
    let store = state.inner.store.clone();
    let info = run_tus_store("create tus upload", move || {
        store.create_upload(&device_id, upload)
    })
    .await?;
    let location = format!("uploads/{}", info.id);
    tus_response(StatusCode::CREATED, |builder| {
        builder.header(LOCATION, location).header(UPLOAD_OFFSET, 0)
    })
}

async fn head_upload(
    State(state): State<ApiState>,
    OriginalUri(uri): OriginalUri,
    Path(parameters): Path<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Response, TusError> {
    let device_id = authorize_transfer(&state, &headers, &uri, "sender")
        .await
        .map_err(TusError::from)?;
    let upload_id = parameters.get("upload_id").cloned().unwrap_or_default();
    require_tus_version(&headers)?;
    let store = state.inner.store.clone();
    let info = run_tus_store("inspect tus upload", move || {
        store.get_upload(&device_id, &upload_id)
    })
    .await?
    .ok_or_else(TusError::not_found)?;
    upload_status_response(StatusCode::OK, &info)
}

async fn patch_upload(
    State(state): State<ApiState>,
    OriginalUri(uri): OriginalUri,
    Path(parameters): Path<HashMap<String, String>>,
    request: Request,
) -> Result<Response, TusError> {
    let device_id = authorize_transfer(&state, request.headers(), &uri, "sender")
        .await
        .map_err(TusError::from)?;
    let upload_id = parameters.get("upload_id").cloned().unwrap_or_default();
    require_tus_version(request.headers())?;
    require_offset_content_type(request.headers())?;
    let expected_offset = required_u64_header(request.headers(), &UPLOAD_OFFSET, "upload_offset")?;

    let store = state.inner.store.clone();
    let queried_device = device_id.clone();
    let queried_id = upload_id.clone();
    let current = run_tus_store("inspect tus upload", move || {
        store.get_upload(&queried_device, &queried_id)
    })
    .await?
    .ok_or_else(TusError::not_found)?;
    if expected_offset != current.offset {
        return Err(TusError::offset_conflict(current.offset));
    }
    let remaining = current.length.saturating_sub(current.offset);
    if let Some(content_length) = optional_u64_header(request.headers(), &CONTENT_LENGTH)?
        && content_length > remaining
    {
        return Err(TusError::too_large(current.offset));
    }

    let standard_file = tempfile::tempfile()
        .map_err(|error| TusError::internal("buffer tus request body", error.into()))?;
    let mut temporary = tokio::fs::File::from_std(standard_file);
    let stream = request
        .into_body()
        .into_data_stream()
        .map_err(io::Error::other);
    let mut reader = StreamReader::new(stream).take(remaining.saturating_add(1));
    let chunk_length = tokio::io::copy(&mut reader, &mut temporary)
        .await
        .map_err(|error| TusError::internal("read tus request body", error.into()))?;
    temporary
        .sync_all()
        .await
        .map_err(|error| TusError::internal("sync buffered tus request body", error.into()))?;
    if chunk_length > remaining {
        return Err(TusError::too_large(current.offset));
    }
    let standard_file = temporary.into_std().await;

    let store = state.inner.store.clone();
    let outcome = run_tus_store("append tus upload", move || {
        store.append_upload(
            &device_id,
            &upload_id,
            expected_offset,
            standard_file,
            chunk_length,
        )
    })
    .await?;
    match outcome {
        AppendUploadOutcome::Appended(info) => {
            upload_status_response(StatusCode::NO_CONTENT, &info)
        }
        AppendUploadOutcome::NotFound => Err(TusError::not_found()),
        AppendUploadOutcome::OffsetMismatch(info) => Err(TusError::offset_conflict(info.offset)),
        AppendUploadOutcome::TooLarge(info) => Err(TusError::too_large(info.offset)),
    }
}

fn parse_upload_metadata(headers: &HeaderMap) -> Result<BTreeMap<String, String>, TusError> {
    let raw = required_header(headers, &UPLOAD_METADATA, "upload_metadata")?;
    if raw.is_empty() || raw.len() > MAX_METADATA_HEADER_BYTES {
        return Err(TusError::bad_request(
            "invalid_upload_metadata",
            "Upload-Metadata is empty or too large",
        ));
    }
    let raw = raw.to_str().map_err(|_| {
        TusError::bad_request(
            "invalid_upload_metadata",
            "Upload-Metadata is not valid ASCII",
        )
    })?;
    let mut metadata = BTreeMap::new();
    for pair in raw.split(',') {
        let pair = pair.trim();
        let (key, encoded) = pair.split_once(' ').ok_or_else(|| {
            TusError::bad_request(
                "invalid_upload_metadata",
                "each Upload-Metadata entry must contain a key and base64 value",
            )
        })?;
        if !matches!(
            key,
            "filename" | "media_type" | "sha256" | "relative_path" | "source_version"
        ) || encoded.is_empty()
            || encoded.contains(char::is_whitespace)
        {
            return Err(TusError::bad_request(
                "invalid_upload_metadata",
                "Upload-Metadata contains an unsupported key or malformed value",
            ));
        }
        let decoded = STANDARD
            .decode(encoded)
            .or_else(|_| STANDARD_NO_PAD.decode(encoded))
            .map_err(|_| {
                TusError::bad_request(
                    "invalid_upload_metadata",
                    "Upload-Metadata contains invalid base64",
                )
            })?;
        if decoded.len() > 1024 {
            return Err(TusError::bad_request(
                "invalid_upload_metadata",
                "Upload-Metadata value exceeds its safety limit",
            ));
        }
        let value = String::from_utf8(decoded).map_err(|_| {
            TusError::bad_request(
                "invalid_upload_metadata",
                "Upload-Metadata values must be UTF-8",
            )
        })?;
        if metadata.insert(key.to_owned(), value).is_some() {
            return Err(TusError::bad_request(
                "invalid_upload_metadata",
                "Upload-Metadata keys must be unique",
            ));
        }
    }
    if !["filename", "media_type", "sha256"]
        .iter()
        .all(|key| metadata.contains_key(*key))
    {
        return Err(TusError::bad_request(
            "invalid_upload_metadata",
            "Upload-Metadata requires filename, media_type, and sha256",
        ));
    }
    Ok(metadata)
}

fn canonical_metadata_header(metadata: &BTreeMap<String, String>) -> String {
    [
        "filename",
        "media_type",
        "sha256",
        "relative_path",
        "source_version",
    ]
    .into_iter()
    .filter(|key| metadata.contains_key(*key))
    .map(|key| {
        format!(
            "{key} {}",
            STANDARD.encode(metadata.get(key).expect("required metadata was checked"))
        )
    })
    .collect::<Vec<_>>()
    .join(",")
}

fn require_tus_version(headers: &HeaderMap) -> Result<(), TusError> {
    if headers.get_all(&TUS_RESUMABLE).iter().count() != 1
        || headers
            .get(&TUS_RESUMABLE)
            .is_none_or(|value| value.as_bytes() != TUS_VERSION_VALUE.as_bytes())
    {
        return Err(TusError::new(
            StatusCode::PRECONDITION_FAILED,
            "unsupported_tus_version",
            "Tus-Resumable must be 1.0.0",
        ));
    }
    Ok(())
}

fn require_offset_content_type(headers: &HeaderMap) -> Result<(), TusError> {
    if headers.get_all(CONTENT_TYPE).iter().count() != 1
        || headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_none_or(|value| !value.eq_ignore_ascii_case(OFFSET_CONTENT_TYPE))
    {
        return Err(TusError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "invalid_patch_content_type",
            "PATCH Content-Type must be application/offset+octet-stream",
        ));
    }
    Ok(())
}

fn required_u64_header(
    headers: &HeaderMap,
    name: &HeaderName,
    code: &'static str,
) -> Result<u64, TusError> {
    let value = required_header(headers, name, code)?;
    parse_u64_header(value, code)
}

fn optional_u64_header(headers: &HeaderMap, name: &HeaderName) -> Result<Option<u64>, TusError> {
    let count = headers.get_all(name).iter().count();
    match count {
        0 => Ok(None),
        1 => headers
            .get(name)
            .map(|value| parse_u64_header(value, "invalid_content_length"))
            .transpose(),
        _ => Err(TusError::bad_request(
            "invalid_content_length",
            "header must appear exactly once",
        )),
    }
}

fn required_header<'a>(
    headers: &'a HeaderMap,
    name: &HeaderName,
    code: &'static str,
) -> Result<&'a HeaderValue, TusError> {
    if headers.get_all(name).iter().count() != 1 {
        return Err(TusError::bad_request(
            code,
            "header must appear exactly once",
        ));
    }
    Ok(headers.get(name).expect("header count was checked"))
}

fn parse_u64_header(value: &HeaderValue, code: &'static str) -> Result<u64, TusError> {
    let value = value
        .to_str()
        .ok()
        .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .ok_or_else(|| TusError::bad_request(code, "header must be an unsigned decimal integer"))?;
    value
        .parse()
        .map_err(|_| TusError::bad_request(code, "header integer is out of range"))
}

fn upload_status_response(status: StatusCode, info: &UploadInfo) -> Result<Response, TusError> {
    tus_response(status, |mut builder| {
        builder = builder
            .header(UPLOAD_OFFSET, info.offset)
            .header(UPLOAD_LENGTH, info.length)
            .header(UPLOAD_METADATA, info.metadata_header.as_str());
        if let Some(delivery_id) = &info.delivery_id {
            builder = builder.header(MIRELAY_DELIVERY_ID, delivery_id.as_str());
        }
        builder
    })
}

fn tus_response(
    status: StatusCode,
    headers: impl FnOnce(axum::http::response::Builder) -> axum::http::response::Builder,
) -> Result<Response, TusError> {
    headers(
        Response::builder()
            .status(status)
            .header(TUS_RESUMABLE, TUS_VERSION_VALUE),
    )
    .body(Body::empty())
    .map_err(|error| TusError::internal("build tus response", error.into()))
}

async fn run_tus_store<T, F>(operation: &'static str, task: F) -> Result<T, TusError>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    match tokio::task::spawn_blocking(task).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error))
            if error
                .chain()
                .any(|cause| cause.is::<super::directory::DirectoryConflict>()) =>
        {
            Err(TusError::new(
                StatusCode::CONFLICT,
                "directory_conflict",
                "Initialize the receiver or refresh the conflicting directory version.",
            ))
        }
        Ok(Err(error))
            if error
                .chain()
                .any(|cause| cause.is::<InvalidUploadContent>()) =>
        {
            Err(TusError::invalid_content())
        }
        Ok(Err(error)) => Err(TusError::internal(operation, error)),
        Err(error) => Err(TusError::internal(operation, error.into())),
    }
}

struct TusError {
    inner: ApiError,
    offset: Option<u64>,
}

impl TusError {
    fn new(status: StatusCode, code: &'static str, detail: &'static str) -> Self {
        Self {
            inner: ApiError::new(status, code, detail),
            offset: None,
        }
    }

    fn bad_request(code: &'static str, detail: &'static str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, detail)
    }

    fn internal(operation: &'static str, error: anyhow::Error) -> Self {
        Self {
            inner: ApiError::internal(operation, error),
            offset: None,
        }
    }

    fn not_found() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "upload_not_found",
            "tus upload was not found",
        )
    }

    fn offset_conflict(offset: u64) -> Self {
        Self {
            inner: ApiError::new(
                StatusCode::CONFLICT,
                "upload_offset_conflict",
                "Upload-Offset does not match the server offset",
            ),
            offset: Some(offset),
        }
    }

    fn too_large(offset: u64) -> Self {
        Self {
            inner: ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "upload_too_large",
                "PATCH body exceeds the remaining upload length",
            ),
            offset: Some(offset),
        }
    }

    fn invalid_content() -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_upload_content",
            "uploaded bytes do not match the declared digest or media type",
        )
    }
}

impl From<ApiError> for TusError {
    fn from(inner: ApiError) -> Self {
        Self {
            inner,
            offset: None,
        }
    }
}

impl IntoResponse for TusError {
    fn into_response(self) -> Response {
        let mut response = self.inner.into_response();
        response
            .headers_mut()
            .insert(TUS_RESUMABLE, HeaderValue::from_static(TUS_VERSION_VALUE));
        response
            .headers_mut()
            .insert(TUS_VERSION, HeaderValue::from_static(TUS_VERSION_VALUE));
        if let Some(offset) = self.offset
            && let Ok(value) = HeaderValue::from_str(&offset.to_string())
        {
            response.headers_mut().insert(UPLOAD_OFFSET, value);
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use axum::http::Request;
    use axum::http::header::AUTHORIZATION;
    use sha2::{Digest, Sha256};
    use tower::ServiceExt;

    use super::*;
    use crate::server::{ApiState, ServerStore};

    const TOKEN: &str = "tus-test-token";

    fn png_bytes() -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(b"MiRelay tus API test payload");
        bytes
    }

    fn metadata(bytes: &[u8]) -> String {
        metadata_with_media_type(bytes, "image/png")
    }

    fn metadata_with_media_type(bytes: &[u8], media_type: &str) -> String {
        let digest = hex::encode(Sha256::digest(bytes));
        format!(
            "filename {},media_type {},sha256 {}",
            STANDARD.encode("payload.dat"),
            STANDARD.encode(media_type),
            STANDARD.encode(digest)
        )
    }

    fn tus_request(method: &str, uri: &str) -> axum::http::request::Builder {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header(TUS_RESUMABLE, TUS_VERSION_VALUE)
    }

    #[tokio::test]
    async fn creation_patch_resume_and_finalize_follow_tus_offsets() {
        let root = tempfile::tempdir().unwrap();
        let store = ServerStore::new(root.path().join("server"), 1024).unwrap();
        store.initialize().unwrap();
        let app = super::super::api::router(
            ApiState::new(store.clone(), "linux".to_owned(), TOKEN).unwrap(),
        );
        let bytes = png_bytes();

        let response = app
            .clone()
            .oneshot(
                tus_request("POST", "/api/v1/uploads")
                    .header(UPLOAD_LENGTH, bytes.len())
                    .header(UPLOAD_METADATA, metadata(&bytes))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let location = response.headers()[LOCATION].to_str().unwrap().to_owned();
        let upload_uri = format!("/api/v1/{location}");

        let split = 11;
        let response = app
            .clone()
            .oneshot(
                tus_request("PATCH", &upload_uri)
                    .header(CONTENT_TYPE, OFFSET_CONTENT_TYPE)
                    .header(UPLOAD_OFFSET, 0)
                    .body(Body::from(bytes[..split].to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(response.headers()[UPLOAD_OFFSET], split.to_string());

        let conflict = app
            .clone()
            .oneshot(
                tus_request("PATCH", &upload_uri)
                    .header(CONTENT_TYPE, OFFSET_CONTENT_TYPE)
                    .header(UPLOAD_OFFSET, 0)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(conflict.status(), StatusCode::CONFLICT);
        assert_eq!(conflict.headers()[UPLOAD_OFFSET], split.to_string());

        let head_response = app
            .clone()
            .oneshot(
                tus_request("HEAD", &upload_uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(head_response.status(), StatusCode::OK);
        assert_eq!(head_response.headers()[UPLOAD_OFFSET], split.to_string());

        let completed = app
            .clone()
            .oneshot(
                tus_request("PATCH", &upload_uri)
                    .header(CONTENT_TYPE, OFFSET_CONTENT_TYPE)
                    .header(UPLOAD_OFFSET, split)
                    .body(Body::from(bytes[split..].to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(completed.status(), StatusCode::NO_CONTENT);
        assert_eq!(completed.headers()[UPLOAD_OFFSET], bytes.len().to_string());
        let delivery_id = completed.headers()[MIRELAY_DELIVERY_ID].to_str().unwrap();
        assert_eq!(
            store.list_pending("linux", 0, None, 50).unwrap().deliveries[0].id,
            delivery_id
        );

        let completed_again = app
            .oneshot(
                tus_request("HEAD", &upload_uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(completed_again.status(), StatusCode::OK);
        assert_eq!(completed_again.headers()[MIRELAY_DELIVERY_ID], delivery_id);
        assert!(
            to_bytes(completed_again.into_body(), 1)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn options_and_version_negotiation_are_tus_compatible() {
        let root = tempfile::tempdir().unwrap();
        let store = ServerStore::new(root.path().join("server"), 1024).unwrap();
        store.initialize().unwrap();
        let app =
            super::super::api::router(ApiState::new(store, "linux".to_owned(), TOKEN).unwrap());

        let options_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/api/v1/uploads")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(options_response.status(), StatusCode::NO_CONTENT);
        assert_eq!(options_response.headers()[TUS_VERSION], TUS_VERSION_VALUE);
        assert_eq!(options_response.headers()[TUS_EXTENSION], "creation");

        let missing_version = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/uploads")
                    .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                    .header(UPLOAD_LENGTH, 10)
                    .header(UPLOAD_METADATA, "ignored eA==")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing_version.status(), StatusCode::PRECONDITION_FAILED);
        assert_eq!(missing_version.headers()[TUS_VERSION], TUS_VERSION_VALUE);
    }

    #[tokio::test]
    async fn content_mismatch_returns_422_and_discards_the_upload() {
        let root = tempfile::tempdir().unwrap();
        let store = ServerStore::new(root.path().join("server"), 1024).unwrap();
        store.initialize().unwrap();
        let app = super::super::api::router(
            ApiState::new(store.clone(), "linux".to_owned(), TOKEN).unwrap(),
        );
        let bytes = b"plain text falsely declared as image/png";

        let created = app
            .clone()
            .oneshot(
                tus_request("POST", "/api/v1/uploads")
                    .header(UPLOAD_LENGTH, bytes.len())
                    .header(UPLOAD_METADATA, metadata(bytes))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        let location = created.headers()[LOCATION].to_str().unwrap();
        let upload_uri = format!("/api/v1/{location}");

        let rejected = app
            .clone()
            .oneshot(
                tus_request("PATCH", &upload_uri)
                    .header(CONTENT_TYPE, OFFSET_CONTENT_TYPE)
                    .header(UPLOAD_OFFSET, 0)
                    .body(Body::from(bytes.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let problem: serde_json::Value =
            serde_json::from_slice(&to_bytes(rejected.into_body(), 64 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(problem["code"], "invalid_upload_content");

        let missing = app
            .oneshot(
                tus_request("HEAD", &upload_uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(store.stats("linux").unwrap().pending, 0);
    }
}
