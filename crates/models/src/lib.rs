//! Velos wire types, generated from `fluorite/velos.fl`.

// fluorite emits `derive_new::new` constructors that take one argument per
// field; for wide envelope structs (e.g. ObjectMeta) this trips
// `clippy::too_many_arguments`. The generated constructors are intentional, so
// the lint is allowed for the generated module only.
#[allow(clippy::too_many_arguments)]
pub mod velos {
    include!(concat!(env!("OUT_DIR"), "/velos/mod.rs"));
}

use std::collections::HashMap;

use chrono::Utc;
use uuid::Uuid;

use velos::{
    Container, ContainerPhase, ContainerSpec, ContainerStatus, ObjectMeta, ResourceReqs,
    RestartPolicy,
};

impl ObjectMeta {
    /// A fresh `ObjectMeta` with a random uid, resource_version 0, and now() timestamp.
    pub fn new_named(name: impl Into<String>) -> Self {
        ObjectMeta::new(
            name.into(),
            Uuid::new_v4(),
            HashMap::new(),
            HashMap::new(),
            0,
            Utc::now(),
            None,
            Vec::new(),
        )
    }
}

impl Container {
    /// A minimal `Pending` container for the given image.
    pub fn pending(name: impl Into<String>, image: impl Into<String>) -> Self {
        Container::new(
            ObjectMeta::new_named(name),
            ContainerSpec::new(
                image.into(),
                Vec::new(),
                HashMap::new(),
                ResourceReqs::new(1, 512 * 1024 * 1024),
                RestartPolicy::Never,
                None,
            ),
            ContainerStatus::new(ContainerPhase::Pending, None, None, None, None, None, None),
        )
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::velos::*;

    #[test]
    fn container_round_trips_in_camel_case() {
        let c = Container::pending("c1", "alpine:latest");

        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"resourceVersion\""), "json was: {json}");
        assert!(json.contains("\"creationTimestamp\""), "json was: {json}");
        assert!(json.contains("\"restartPolicy\""), "json was: {json}");

        let back: Container = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn watch_event_is_adjacently_tagged() {
        let ev = WatchEvent::Added(serde_json::json!({ "metadata": { "name": "c1" } }));
        let json = serde_json::to_string(&ev).unwrap();
        // adjacently tagged: { "type": "Added", "object": { ... } }
        assert!(json.contains("\"type\":\"Added\""), "json was: {json}");
        assert!(json.contains("\"object\""), "json was: {json}");

        let back: WatchEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);
    }
}
