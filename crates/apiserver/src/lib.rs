//! Velos API server: a Kubernetes-shaped REST surface over `velos_store`.
//!
//! Objects are handled as opaque JSON; only the indexed envelope fields
//! (`metadata.name`, `metadata.labels`, `spec.nodeName`) are interpreted.
//! Typed admission against `velos-models` is a later phase.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde_json::Value;
use uuid::Uuid;

use velos_store::{Selector, Store, StoredObject};

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

impl From<velos_store::StoreError> for ApiError {
    fn from(e: velos_store::StoreError) -> Self {
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
    doc.get("metadata")?
        .get("name")?
        .as_str()
        .map(str::to_string)
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
    doc.get("spec")?
        .get("nodeName")?
        .as_str()
        .map(str::to_string)
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
        m.entry("finalizers")
            .or_insert_with(|| serde_json::json!([]));
    }
}

fn parse_selector(params: &HashMap<String, String>) -> Result<Selector, ApiError> {
    let mut sel = Selector::default();
    if let Some(ls) = params.get("labelSelector") {
        for pair in ls.split(',').filter(|s| !s.is_empty()) {
            let (k, v) = pair
                .split_once('=')
                .ok_or_else(|| ApiError::BadRequest(format!("bad labelSelector: {pair}")))?;
            sel.labels.push((k.to_string(), v.to_string()));
        }
    }
    if let Some(fs) = params.get("fieldSelector") {
        for pair in fs.split(',').filter(|s| !s.is_empty()) {
            let (k, v) = pair
                .split_once('=')
                .ok_or_else(|| ApiError::BadRequest(format!("bad fieldSelector: {pair}")))?;
            if k == "spec.nodeName" {
                sel.node_name = Some(v.to_string());
            } else {
                return Err(ApiError::BadRequest(format!(
                    "unsupported fieldSelector: {k}"
                )));
            }
        }
    }
    Ok(sel)
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

async fn list(
    State(state): State<AppState>,
    Path(plural): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    let kind = kind_for(&plural).ok_or(ApiError::NotFound)?;
    let selector = parse_selector(&params)?;
    let objs = state.store.list(kind, &selector)?;
    let items: Vec<Value> = objs.into_iter().map(|o| o.document).collect();
    Ok(Json(serde_json::json!({ "items": items })))
}

async fn replace(
    State(state): State<AppState>,
    Path((plural, name)): Path<(String, String)>,
    Json(mut body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let kind = kind_for(&plural).ok_or(ApiError::NotFound)?;
    if !body.is_object() {
        return Err(ApiError::BadRequest("body must be a JSON object".into()));
    }
    let existing = state.store.get(kind, &name)?.ok_or(ApiError::NotFound)?;
    let rv = state.store.next_resource_version()?;
    stamp_meta(&mut body, &existing.uid, rv);

    // Force name to match the path and preserve the original creationTimestamp.
    if let Some(m) = body.get_mut("metadata").and_then(Value::as_object_mut) {
        m.insert("name".to_string(), serde_json::json!(name));
        if let Some(ct) = existing
            .document
            .get("metadata")
            .and_then(|x| x.get("creationTimestamp"))
        {
            m.insert("creationTimestamp".to_string(), ct.clone());
        }
    }

    let obj = StoredObject {
        kind: kind.to_string(),
        name: name.clone(),
        uid: existing.uid,
        resource_version: rv,
        node_name: extract_node_name(&body),
        labels: extract_labels(&body),
        document: body.clone(),
    };
    state.store.put(&obj)?;
    Ok(Json(body))
}

async fn replace_status(
    State(state): State<AppState>,
    Path((plural, name)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let kind = kind_for(&plural).ok_or(ApiError::NotFound)?;
    let mut existing = state.store.get(kind, &name)?.ok_or(ApiError::NotFound)?;
    // Accept either `{ "status": {...} }` or a bare status object.
    let new_status = body.get("status").cloned().unwrap_or(body);
    let rv = state.store.next_resource_version()?;

    if let Some(m) = existing.document.as_object_mut() {
        m.insert("status".to_string(), new_status);
    }
    if let Some(m) = existing
        .document
        .get_mut("metadata")
        .and_then(Value::as_object_mut)
    {
        m.insert("resourceVersion".to_string(), serde_json::json!(rv));
    }
    existing.resource_version = rv;
    state.store.put(&existing)?;
    Ok(Json(existing.document))
}

async fn delete(
    State(state): State<AppState>,
    Path((plural, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let kind = kind_for(&plural).ok_or(ApiError::NotFound)?;
    if state.store.delete(kind, &name)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

pub fn app(store: Arc<dyn Store>) -> Router {
    let state = AppState { store };
    Router::new()
        .route("/api/v1/:plural", post(create).get(list))
        .route(
            "/api/v1/:plural/:name",
            get(get_one).put(replace).delete(delete),
        )
        .route("/api/v1/:plural/:name/status", put(replace_status))
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
        let store = Arc::new(velos_store::SqliteStore::in_memory().unwrap());
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

    async fn post(app: &axum::Router, plural: &str, body: serde_json::Value) {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/{plural}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn list_returns_items_and_honors_selectors() {
        let app = test_app();
        post(
            &app,
            "containers",
            serde_json::json!({
                "metadata": { "name": "c1", "labels": { "team": "a" } },
                "spec": { "image": "img", "nodeName": "node-7" }
            }),
        )
        .await;
        post(
            &app,
            "containers",
            serde_json::json!({
                "metadata": { "name": "c2", "labels": { "team": "b" } },
                "spec": { "image": "img", "nodeName": "node-8" }
            }),
        )
        .await;

        // list all
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/containers")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let all = body_json(resp).await;
        assert_eq!(all["items"].as_array().unwrap().len(), 2);

        // label selector
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/containers?labelSelector=team=a")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let filtered = body_json(resp).await;
        assert_eq!(filtered["items"].as_array().unwrap().len(), 1);
        assert_eq!(filtered["items"][0]["metadata"]["name"], "c1");

        // field selector
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/containers?fieldSelector=spec.nodeName=node-8")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let by_node = body_json(resp).await;
        assert_eq!(by_node["items"].as_array().unwrap().len(), 1);
        assert_eq!(by_node["items"][0]["metadata"]["name"], "c2");
    }

    #[tokio::test]
    async fn replace_status_and_delete_lifecycle() {
        let app = test_app();
        post(
            &app,
            "containers",
            serde_json::json!({
                "metadata": { "name": "c1" },
                "spec": { "image": "img" }
            }),
        )
        .await;

        // PUT status subresource
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/containers/c1/status")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "status": { "phase": "Running" } }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let after = body_json(resp).await;
        assert_eq!(after["status"]["phase"], "Running");
        assert_eq!(after["spec"]["image"], "img"); // spec preserved
        assert_eq!(after["metadata"]["resourceVersion"], 2); // bumped

        // PUT replace whole object
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/containers/c1")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "metadata": { "name": "c1" },
                            "spec": { "image": "img2" }
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let replaced = body_json(resp).await;
        assert_eq!(replaced["spec"]["image"], "img2");
        assert_eq!(replaced["metadata"]["resourceVersion"], 3);

        // DELETE
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/v1/containers/c1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // DELETE again → 404
        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/v1/containers/c1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
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
