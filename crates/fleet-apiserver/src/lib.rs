//! Fleet API server: a Kubernetes-shaped REST surface over `fleet_store`.
//!
//! Objects are handled as opaque JSON; only the indexed envelope fields
//! (`metadata.name`, `metadata.labels`, `spec.nodeName`) are interpreted.
//! Typed admission against `fleet-models` is a later phase.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::Value;
use uuid::Uuid;

use fleet_store::{Store, StoredObject};

#[derive(Clone)]
struct AppState {
    store: Arc<dyn Store>,
}

pub enum ApiError {
    NotFound,
    BadRequest(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        };
        (status, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}

impl From<fleet_store::StoreError> for ApiError {
    fn from(e: fleet_store::StoreError) -> Self {
        ApiError::Internal(e.to_string())
    }
}

/// Maps a plural resource path segment to its stored kind.
fn kind_for(plural: &str) -> Option<&'static str> {
    match plural {
        "containers" => Some("Container"),
        "workers" => Some("Worker"),
        "leases" => Some("Lease"),
        _ => None,
    }
}

fn extract_name(doc: &Value) -> Option<String> {
    doc.get("metadata")?.get("name")?.as_str().map(str::to_string)
}

fn extract_labels(doc: &Value) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Some(map) = doc
        .get("metadata")
        .and_then(|m| m.get("labels"))
        .and_then(Value::as_object)
    {
        for (k, v) in map {
            if let Some(s) = v.as_str() {
                out.insert(k.clone(), s.to_string());
            }
        }
    }
    out
}

fn extract_node_name(doc: &Value) -> Option<String> {
    doc.get("spec")?.get("nodeName")?.as_str().map(str::to_string)
}

/// Stamps server-owned envelope fields into a (mutable) object document.
fn stamp_meta(doc: &mut Value, uid: &Uuid, rv: u64) {
    if !doc.get("metadata").map(Value::is_object).unwrap_or(false) {
        doc["metadata"] = serde_json::json!({});
    }
    if let Some(m) = doc.get_mut("metadata").and_then(Value::as_object_mut) {
        m.insert("uid".to_string(), serde_json::json!(uid.to_string()));
        m.insert("resourceVersion".to_string(), serde_json::json!(rv));
        m.entry("creationTimestamp")
            .or_insert_with(|| serde_json::json!(chrono::Utc::now().to_rfc3339()));
        m.entry("labels").or_insert_with(|| serde_json::json!({}));
        m.entry("annotations")
            .or_insert_with(|| serde_json::json!({}));
        m.entry("finalizers").or_insert_with(|| serde_json::json!([]));
    }
}

async fn create(
    State(state): State<AppState>,
    Path(plural): Path<String>,
    Json(mut body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let kind = kind_for(&plural).ok_or(ApiError::NotFound)?;
    if !body.is_object() {
        return Err(ApiError::BadRequest("body must be a JSON object".into()));
    }
    let name =
        extract_name(&body).ok_or_else(|| ApiError::BadRequest("metadata.name required".into()))?;
    let uid = Uuid::new_v4();
    let rv = state.store.next_resource_version()?;
    stamp_meta(&mut body, &uid, rv);

    let obj = StoredObject {
        kind: kind.to_string(),
        name,
        uid,
        resource_version: rv,
        node_name: extract_node_name(&body),
        labels: extract_labels(&body),
        document: body.clone(),
    };
    state.store.put(&obj)?;
    Ok((StatusCode::CREATED, Json(body)))
}

async fn get_one(
    State(state): State<AppState>,
    Path((plural, name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let kind = kind_for(&plural).ok_or(ApiError::NotFound)?;
    let obj = state.store.get(kind, &name)?.ok_or(ApiError::NotFound)?;
    Ok(Json(obj.document))
}

pub fn app(store: Arc<dyn Store>) -> Router {
    let state = AppState { store };
    Router::new()
        .route("/api/v1/:plural", post(create))
        .route("/api/v1/:plural/:name", get(get_one))
        .with_state(state)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::sync::Arc;
    use tower::ServiceExt;

    fn test_app() -> axum::Router {
        let store = Arc::new(fleet_store::SqliteStore::in_memory().unwrap());
        app(store)
    }

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn create_assigns_uid_and_resource_version_then_get_returns_it() {
        let app = test_app();
        let body = serde_json::json!({
            "metadata": { "name": "c1" },
            "spec": { "image": "alpine:latest" }
        });

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/containers")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let created = body_json(resp).await;
        assert_eq!(created["metadata"]["resourceVersion"], 1);
        assert!(created["metadata"]["uid"].is_string());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/containers/c1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let got = body_json(resp).await;
        assert_eq!(got["metadata"]["name"], "c1");
        assert_eq!(got["spec"]["image"], "alpine:latest");
    }

    #[tokio::test]
    async fn create_without_name_is_bad_request() {
        let app = test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/containers")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::json!({ "spec": {} }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn get_unknown_kind_is_not_found() {
        let app = test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/widgets/x")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
