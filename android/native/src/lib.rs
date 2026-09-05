use std::panic::{AssertUnwindSafe, catch_unwind};

use jni::JNIEnv;
use jni::objects::{JClass, JObject, JString, JValue};
use jni::sys::jstring;
use mirelay::tus_client::{
    UploadEvent, UploadRequest, is_retryable_upload_error, upload_with_events,
};
use serde_json::json;

/// Directory inventories stay in Rust's bounded, scoped protocol. Source URIs
/// remain local; credentials are never included in previews or inventories.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_mirelay_android_NativeBridge_directory(
    mut env: JNIEnv,
    _class: JClass,
    input: JString,
) -> jstring {
    let outcome = catch_unwind(AssertUnwindSafe(|| -> Result<serde_json::Value, String> {
        let text:String = env.get_string(&input).map_err(|_| "Invalid directory request.")?.into();
        if text.len() > mirelay::directory::MAX_BODY { return Err("Directory request is too large.".into()); }
        let value:serde_json::Value = serde_json::from_str(&text).map_err(|_| "Invalid directory request.")?;
        match value["action"].as_str() {
            Some("state") => {
                let client = mirelay::directory::client::DirectoryClient::new(
                    value["server_url"].as_str().ok_or("Missing server URL.")?,
                    value["token"].as_str().ok_or("Missing credential.")?,
                    value["allow_insecure_http"].as_bool().unwrap_or(false), "sender",
                ).map_err(|e|e.to_string())?;
                Ok(json!({"result":client.state().map_err(|e|e.to_string())?}))
            }
            Some("compare") => {
                let source = serde_json::from_value(value["source"].clone()).map_err(|_| "Invalid source inventory.")?;
                let remote = serde_json::from_value(value["remote"].clone()).map_err(|_| "Invalid receiver inventory.")?;
                Ok(json!({"result":mirelay::directory::compare_remote(&source,&remote).map_err(|e|e.to_string())?}))
            }
            _ => Err("Unsupported directory operation.".into()),
        }
    })).unwrap_or_else(|_|Err("Directory operation stopped safely.".into()));
    let value = outcome.unwrap_or_else(|error| json!({"error":error}));
    if env.exception_check().unwrap_or(true) {
        return std::ptr::null_mut();
    }
    env.new_string(value.to_string())
        .map(JString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_mirelay_android_NativeBridge_pairing(
    mut env: JNIEnv,
    _class: JClass,
    input: JString,
) -> jstring {
    let outcome = catch_unwind(AssertUnwindSafe(|| -> Result<serde_json::Value, String> {
        let text: String = env
            .get_string(&input)
            .map_err(|_| "Invalid setup request.".to_owned())?
            .into();
        if text.len() > 16 * 1024 {
            return Err("Setup request is too large.".into());
        }
        let value: serde_json::Value =
            serde_json::from_str(&text).map_err(|_| "Invalid setup request.".to_owned())?;
        let server = value["server_url"].as_str().ok_or("Missing server URL.")?;
        let token = value["token"].as_str().ok_or("Missing credential.")?;
        let client = mirelay::pairing::PairingClient::new(
            server,
            token,
            value["allow_insecure_http"].as_bool().unwrap_or(false),
        )
        .map_err(|e| e.to_string())?;
        let info = match value["action"].as_str() {
            Some("claim") => client
                .claim(
                    value["pairing_code"]
                        .as_str()
                        .ok_or("Missing pairing code.")?,
                    token,
                )
                .map(Some),
            Some("handshake") => client.handshake().map(Some),
            Some("legacy_check") => client.verify_receiver(),
            _ => return Err("Unsupported setup operation.".into()),
        }
        .map_err(|e| e.to_string())?;
        Ok(json!({"result": info}))
    }))
    .unwrap_or_else(|_| Err("Native setup stopped safely.".into()));
    let value = outcome.unwrap_or_else(|error| json!({"error": error}));
    if env.exception_check().unwrap_or(true) {
        return std::ptr::null_mut();
    }
    env.new_string(value.to_string())
        .map(JString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_mirelay_android_NativeBridge_initialize(
    mut env: jni22::EnvUnowned<'_>,
    _class: jni22::objects::JClass<'_>,
    context: jni22::objects::JObject<'_>,
) -> jni22::sys::jboolean {
    let outcome =
        env.with_env(|env| rustls_platform_verifier::android::init_with_env(env, context));
    matches!(outcome.into_outcome(), jni22::Outcome::Ok(()))
}

#[cfg(not(target_os = "android"))]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_mirelay_android_NativeBridge_initialize(
    _env: JNIEnv,
    _class: JClass,
    _context: JObject,
) -> jni::sys::jboolean {
    1
}

/// Synchronous JNI boundary: the caller must use a background worker.
/// No Rust panic is allowed to unwind into ART/JVM frames.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_mirelay_android_NativeBridge_upload(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
    progress: JObject,
) -> jstring {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let text: String = match env.get_string(&request) {
            Ok(text) => text.into(),
            Err(_) => return json!({"error": "Invalid native request", "retryable": false}),
        };
        if text.len() > 64 * 1024 {
            return json!({"error": "Native request is too large", "retryable": false});
        }
        let request: UploadRequest = match serde_json::from_str(&text) {
            Ok(request) => request,
            Err(_) => return json!({"error": "Invalid upload configuration", "retryable": false}),
        };
        let token = request.token.clone();
        match upload_with_events(request, |event| {
            if let UploadEvent::Progress { uploaded, total } = event {
                env.call_method(&progress, "update", "(JJ)Z", &[
                    JValue::Long(*uploaded as i64), JValue::Long(*total as i64),
                ]).and_then(|value| value.z()).unwrap_or(false)
            } else {
                true
            }
        }) {
            Ok(outcome) => json!({"result": outcome}),
            Err(error) => {
                let retryable = is_retryable_upload_error(&error);
                // A server's error body is untrusted and could echo credentials.
                let message = format!("{error:#}");
                let message = if token.is_empty() { message } else { message.replace(&token, "[redacted]") };
                json!({"error": message.chars().take(1000).collect::<String>(), "retryable": retryable})
            }
        }
    })).unwrap_or_else(|_| json!({"error": "Native upload failed safely", "retryable": false}));
    if env.exception_check().unwrap_or(true) {
        return std::ptr::null_mut();
    }
    env.new_string(result.to_string())
        .map(JString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}
