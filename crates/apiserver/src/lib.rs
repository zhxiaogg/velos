//! Velos API server: a Kubernetes-shaped REST surface over `velos_store`.
//!
//! Objects are handled as opaque JSON; only the indexed envelope fields
//! (`metadata.name`, `metadata.labels`, `spec.nodeName`) are interpreted.
//! Typed admission against `velos-models` is a later phase.

pub mod controllers;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde_json::Value;
use uuid::Uuid;
use velos_auth::{AuthService, Identity};
use velos_store::{EventType, Selector, Store, StoreError, StoredEvent, StoredObject};

/// Poll interval for the watch event log.
const WATCH_POLL: Duration = Duration::from_millis(100);

#[derive(Clone)]
pub struct AppState {
    store: Arc<dyn Store>,
    auth: Option<Arc<dyn AuthService>>,
}

pub enum ApiError {
    NotFound,
    BadRequest(String),
    Unauthorized,
    Forbidden,
    Conflict(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized".to_string()),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, "forbidden".to_string()),
            ApiError::Conflict(m) => (StatusCode::CONFLICT, m),
            ApiError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        };
        (status, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        match &e {
            StoreError::Conflict { .. } => ApiError::Conflict(e.to_string()),
            StoreError::Sqlite(_)
            | StoreError::Serde(_)
            | StoreError::Uid(_)
            | StoreError::Lock => ApiError::Internal(e.to_string()),
        }
    }
}

/// JSON name of a `WatchEvent` variant, matching the fluorite-generated tag.
fn event_type_name(t: EventType) -> &'static str {
    match t {
        EventType::Added => "Added",
        EventType::Modified => "Modified",
        EventType::Deleted => "Deleted",
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

async fn list_or_watch(
    State(state): State<AppState>,
    Path(plural): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, ApiError> {
    let kind = kind_for(&plural).ok_or(ApiError::NotFound)?;
    if params.get("watch").map(|v| v == "true").unwrap_or(false) {
        return Ok(watch(state, kind, &params));
    }
    let selector = parse_selector(&params)?;
    let objs = state.store.list(kind, &selector)?;
    let items: Vec<Value> = objs.into_iter().map(|o| o.document).collect();
    Ok(Json(serde_json::json!({ "items": items })).into_response())
}

/// Render one event-log entry as an NDJSON `WatchEvent` frame line.
fn watch_frame(ev: &StoredEvent) -> String {
    let frame = serde_json::json!({
        "type": event_type_name(ev.event_type),
        "object": ev.document,
    });
    // serde_json on a Value never fails; fall back to an empty object on the
    // impossible error path rather than panicking.
    serde_json::to_string(&frame).unwrap_or_else(|_| "{}".to_string()) + "\n"
}

/// Stream `WatchEvent` frames as chunked NDJSON: replay the event log from
/// `resourceVersion`, then poll for live events. `watchTimeoutSeconds` bounds the
/// stream (used by clients that want a finite watch; absent → runs until the
/// connection drops).
fn watch(state: AppState, kind: &'static str, params: &HashMap<String, String>) -> Response {
    let since = params
        .get("resourceVersion")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    let deadline = params
        .get("watchTimeoutSeconds")
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs);

    let stream = async_stream::stream! {
        let mut last = since;
        let mut elapsed = Duration::ZERO;
        loop {
            match state.store.list_since(kind, last) {
                Ok(events) => {
                    for ev in events {
                        if ev.resource_version > last {
                            last = ev.resource_version;
                        }
                        yield Ok::<_, std::io::Error>(watch_frame(&ev).into_bytes());
                    }
                }
                Err(_) => break,
            }
            if let Some(d) = deadline
                && elapsed >= d
            {
                break;
            }
            tokio::time::sleep(WATCH_POLL).await;
            elapsed += WATCH_POLL;
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// True when a document carries at least one finalizer.
fn has_finalizers(doc: &Value) -> bool {
    doc.get("metadata")
        .and_then(|m| m.get("finalizers"))
        .and_then(Value::as_array)
        .map(|a| !a.is_empty())
        .unwrap_or(false)
}

/// True when a document is marked for deletion (`deletionTimestamp` set).
fn is_marked_for_deletion(doc: &Value) -> bool {
    doc.get("metadata")
        .and_then(|m| m.get("deletionTimestamp"))
        .map(|v| !v.is_null())
        .unwrap_or(false)
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
    // Capture the client's optimistic-concurrency precondition before re-stamping.
    let precondition = body
        .get("metadata")
        .and_then(|m| m.get("resourceVersion"))
        .and_then(Value::as_u64);

    let existing = state.store.get(kind, &name)?.ok_or(ApiError::NotFound)?;
    let rv = state.store.next_resource_version()?;
    stamp_meta(&mut body, &existing.uid, rv);

    // Force name to match the path and preserve server-owned timestamps.
    if let Some(m) = body.get_mut("metadata").and_then(Value::as_object_mut) {
        m.insert("name".to_string(), serde_json::json!(name));
        if let Some(ct) = existing
            .document
            .get("metadata")
            .and_then(|x| x.get("creationTimestamp"))
        {
            m.insert("creationTimestamp".to_string(), ct.clone());
        }
        // Preserve an existing deletionTimestamp unless the client cleared it.
        if !m.contains_key("deletionTimestamp")
            && let Some(dt) = existing
                .document
                .get("metadata")
                .and_then(|x| x.get("deletionTimestamp"))
            && !dt.is_null()
        {
            m.insert("deletionTimestamp".to_string(), dt.clone());
        }
    }

    // Finalizer protocol: once marked for deletion and the last finalizer is
    // cleared, the apiserver hard-deletes (and emits a Deleted event).
    if is_marked_for_deletion(&body) && !has_finalizers(&body) {
        state.store.delete(kind, &name)?;
        return Ok(Json(body));
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
    match precondition {
        Some(expected) => state.store.put_cas(&obj, expected)?,
        None => state.store.put(&obj)?,
    }
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
) -> Result<Response, ApiError> {
    let kind = kind_for(&plural).ok_or(ApiError::NotFound)?;
    let existing = state.store.get(kind, &name)?.ok_or(ApiError::NotFound)?;

    // With finalizers present, mark for deletion instead of removing; the owning
    // controller clears its finalizer, after which `replace` hard-deletes.
    if has_finalizers(&existing.document) {
        let rv = state.store.next_resource_version()?;
        let mut doc = existing.document.clone();
        if let Some(m) = doc.get_mut("metadata").and_then(Value::as_object_mut) {
            m.insert("resourceVersion".to_string(), serde_json::json!(rv));
            m.entry("deletionTimestamp")
                .or_insert_with(|| serde_json::json!(chrono::Utc::now().to_rfc3339()));
        }
        let obj = StoredObject {
            kind: kind.to_string(),
            name: name.clone(),
            uid: existing.uid,
            resource_version: rv,
            node_name: existing.node_name.clone(),
            labels: existing.labels.clone(),
            document: doc.clone(),
        };
        state.store.put(&obj)?;
        return Ok((StatusCode::OK, Json(doc)).into_response());
    }

    match state.store.delete(kind, &name)? {
        Some(_) => Ok(StatusCode::NO_CONTENT.into_response()),
        None => Err(ApiError::NotFound),
    }
}

// ---------------------------------------------------------------------------
// Auth: bootstrap-token mint + worker registration + request authentication.
// ---------------------------------------------------------------------------

fn bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::to_string)
}

/// `POST /auth/v1/tokens` — mint a bootstrap token. Body: `{ "ttlSeconds": N }`.
async fn mint_token(
    State(state): State<AppState>,
    body: Option<Json<Value>>,
) -> Result<Json<Value>, ApiError> {
    let auth = state.auth.as_ref().ok_or(ApiError::NotFound)?;
    let ttl = body
        .and_then(|Json(b)| b.get("ttlSeconds").and_then(Value::as_i64))
        .unwrap_or(24 * 3600);
    let tok = auth
        .mint_bootstrap_token(ttl)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({
        "tokenId": tok.token_id,
        "secret": tok.secret.expose(),
        "expiresAt": tok.expires_at.to_rfc3339(),
    })))
}

/// `POST /auth/v1/register` — join with a bootstrap token, get a credential.
async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let auth = state.auth.as_ref().ok_or(ApiError::NotFound)?;
    let token = bearer(&headers).ok_or(ApiError::Unauthorized)?;
    auth.verify_bootstrap(&token)
        .map_err(|_| ApiError::Unauthorized)?;

    let name = req
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::BadRequest("name required".into()))?
        .to_string();
    let capacity = req
        .get("capacity")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let addresses = req
        .get("addresses")
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    let runtime_version = req
        .get("containerRuntimeVersion")
        .cloned()
        .unwrap_or_else(|| serde_json::json!("unknown"));

    let uid = Uuid::new_v4();
    let rv = state.store.next_resource_version()?;
    let mut doc = serde_json::json!({
        "metadata": { "name": name },
        "spec": { "unschedulable": false },
        "status": {
            "capacity": capacity,
            "allocatable": capacity,
            "conditions": [],
            "addresses": addresses,
            "containerRuntimeVersion": runtime_version,
        }
    });
    stamp_meta(&mut doc, &uid, rv);
    state.store.put(&StoredObject {
        kind: "Worker".to_string(),
        name: name.clone(),
        uid,
        resource_version: rv,
        node_name: None,
        labels: HashMap::new(),
        document: doc,
    })?;

    let credential = auth
        .issue_credential(&name)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({
        "workerName": name,
        "token": credential,
    })))
}

/// Authenticate every `/api/v1` request and enforce worker-scoped access.
async fn require_auth(
    State(auth): State<Arc<dyn AuthService>>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let token = bearer(request.headers()).ok_or(ApiError::Unauthorized)?;
    let Identity::Worker(who) = auth.authenticate(&token).ok_or(ApiError::Unauthorized)?;

    // A worker may only address its own Worker and Lease objects by name.
    // Container access is allowed for any authenticated worker (nodeName-scoped
    // enforcement is a documented refinement).
    let path = request.uri().path();
    if let Some((plural, name)) = named_path(path)
        && matches!(plural, "workers" | "leases")
        && name != who
    {
        return Err(ApiError::Forbidden);
    }
    Ok(next.run(request).await)
}

/// Extract `(plural, name)` from `/api/v1/{plural}/{name}[/...]`, if present.
fn named_path(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("/api/v1/")?;
    let mut parts = rest.split('/');
    let plural = parts.next()?;
    let name = parts.next()?;
    if name.is_empty() {
        return None;
    }
    Some((plural, name))
}

fn api_routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/:plural", post(create).get(list_or_watch))
        .route(
            "/api/v1/:plural/:name",
            get(get_one).put(replace).delete(delete),
        )
        .route("/api/v1/:plural/:name/status", put(replace_status))
}

/// Build the apiserver with no authentication (dev / tests / e2e).
pub fn app(store: Arc<dyn Store>) -> Router {
    let state = AppState { store, auth: None };
    api_routes().with_state(state)
}

/// Build the apiserver with bootstrap-token auth: `/auth/v1` endpoints are open
/// (they self-verify), while every `/api/v1` request must present a valid worker
/// credential and may only touch its own Worker/Lease objects.
pub fn app_with_auth(store: Arc<dyn Store>, auth: Arc<dyn AuthService>) -> Router {
    let state = AppState {
        store,
        auth: Some(Arc::clone(&auth)),
    };
    let protected = api_routes().layer(middleware::from_fn_with_state(
        Arc::clone(&auth),
        require_auth,
    ));
    Router::new()
        .route("/auth/v1/tokens", post(mint_token))
        .route("/auth/v1/register", post(register))
        .merge(protected)
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
    async fn replace_with_stale_resource_version_conflicts() {
        let app = test_app();
        post(
            &app,
            "containers",
            serde_json::json!({ "metadata": { "name": "c1" }, "spec": { "image": "img" } }),
        )
        .await;

        // First replace with precondition rv=1 succeeds (object is at rv 1).
        let ok = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/containers/c1")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "metadata": { "name": "c1", "resourceVersion": 1 },
                            "spec": { "image": "img2" }
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);

        // Second replace reuses the now-stale precondition rv=1 → 409.
        let conflict = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/containers/c1")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "metadata": { "name": "c1", "resourceVersion": 1 },
                            "spec": { "image": "img3" }
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(conflict.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn delete_with_finalizer_marks_then_hard_deletes_when_cleared() {
        let app = test_app();
        post(
            &app,
            "containers",
            serde_json::json!({
                "metadata": { "name": "c1", "finalizers": ["veloslet"] },
                "spec": { "image": "img" }
            }),
        )
        .await;

        // DELETE with a finalizer present → object is marked, not removed.
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
        assert_eq!(resp.status(), StatusCode::OK);
        let marked = body_json(resp).await;
        assert!(marked["metadata"]["deletionTimestamp"].is_string());

        // Still retrievable.
        let resp = app
            .clone()
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

        // Clear the finalizer via replace → server hard-deletes.
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/containers/c1")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "metadata": { "name": "c1", "finalizers": [] },
                            "spec": { "image": "img" }
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

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
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn watch_streams_added_and_modified_frames() {
        let app = test_app();
        post(
            &app,
            "containers",
            serde_json::json!({ "metadata": { "name": "c1" }, "spec": { "image": "img" } }),
        )
        .await;
        // status write produces a Modified event
        let _ = app
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

        // watch from version 0 with a short timeout so the stream terminates.
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/containers?watch=true&resourceVersion=0&watchTimeoutSeconds=1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        let frames: Vec<serde_json::Value> = text
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(frames.len(), 2, "frames: {text}");
        assert_eq!(frames[0]["type"], "Added");
        assert_eq!(frames[1]["type"], "Modified");
        assert_eq!(frames[1]["object"]["status"]["phase"], "Running");
    }

    async fn send(app: &axum::Router, req: Request<Body>) -> axum::response::Response {
        app.clone().oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn auth_flow_mint_register_then_scoped_access() {
        let store: Arc<dyn velos_store::Store> =
            Arc::new(velos_store::SqliteStore::in_memory().unwrap());
        let auth = Arc::new(velos_auth::StoreAuthenticator::new(Arc::clone(&store)));
        let app = app_with_auth(store, auth);

        // Mint a bootstrap token.
        let resp = send(
            &app,
            Request::builder()
                .method("POST")
                .uri("/auth/v1/tokens")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "ttlSeconds": 60 }).to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let tok = body_json(resp).await;
        let boot = format!(
            "{}.{}",
            tok["tokenId"].as_str().unwrap(),
            tok["secret"].as_str().unwrap()
        );

        // Register with the bootstrap token → receive a worker credential.
        let resp = send(
            &app,
            Request::builder()
                .method("POST")
                .uri("/auth/v1/register")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {boot}"))
                .body(Body::from(
                    serde_json::json!({
                        "name": "w1",
                        "capacity": { "cpu": 4, "memoryBytes": 8589934592u64, "maxContainers": 8 },
                        "containerRuntimeVersion": "fake/1.0"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let cred = body_json(resp).await;
        let credential = cred["token"].as_str().unwrap().to_string();

        // No credential → 401.
        let resp = send(
            &app,
            Request::builder()
                .method("GET")
                .uri("/api/v1/workers/w1")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // Valid credential, own Worker → 200.
        let resp = send(
            &app,
            Request::builder()
                .method("GET")
                .uri("/api/v1/workers/w1")
                .header("authorization", format!("Bearer {credential}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);

        // Valid credential, another worker's Lease → 403.
        let resp = send(
            &app,
            Request::builder()
                .method("GET")
                .uri("/api/v1/leases/other")
                .header("authorization", format!("Bearer {credential}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
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
