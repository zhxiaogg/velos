//! `velosctl` REST helpers (the kubectl analog). The pure URL/kind helpers live
//! here so they can be unit-tested; the binary in `main.rs` wires them to reqwest.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const DEFAULT_SERVER: &str = "http://127.0.0.1:8080";

/// Persisted velosctl credentials (`~/.velos/config`), written by `login`.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

/// Resolve the apiserver URL: flag > env > config > built-in default.
pub fn resolve_server(flag: Option<&str>, env: Option<&str>, cfg: &Config) -> String {
    flag.map(str::to_string)
        .or_else(|| env.map(str::to_string))
        .or_else(|| cfg.server.clone())
        .unwrap_or_else(|| DEFAULT_SERVER.to_string())
}

/// Resolve the bearer token: flag > env > config.
pub fn resolve_token(flag: Option<&str>, env: Option<&str>, cfg: &Config) -> Option<String> {
    flag.map(str::to_string)
        .or_else(|| env.map(str::to_string))
        .or_else(|| cfg.token.clone())
}

/// `~/.velos/config`, if a home directory is known.
pub fn config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".velos").join("config"))
}

/// Load config; any error (missing / unreadable / garbage) yields defaults.
pub fn load_config() -> Config {
    let Some(p) = config_path() else {
        return Config::default();
    };
    match fs::read_to_string(&p) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

/// Persist config to `~/.velos/config` with 0700 dir / 0600 file perms.
pub fn save_config(cfg: &Config) -> std::io::Result<()> {
    let Some(p) = config_path() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no home directory",
        ));
    };
    if let Some(dir) = p.parent() {
        fs::create_dir_all(dir)?;
        set_mode(dir, 0o700);
    }
    let body = serde_json::to_string_pretty(cfg).unwrap_or_else(|_| "{}".to_string());
    fs::write(&p, body)?;
    set_mode(&p, 0o600);
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &std::path::Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn set_mode(_path: &std::path::Path, _mode: u32) {}

/// Map a singular or plural resource word to its REST path segment.
/// Returns `None` for unknown kinds (fail closed).
pub fn plural_for(kind: &str) -> Option<&'static str> {
    match kind {
        "container" | "containers" => Some("containers"),
        "worker" | "workers" => Some("workers"),
        "lease" | "leases" => Some("leases"),
        _ => None,
    }
}

/// Build a collection URL, optionally with a label selector.
pub fn collection_url(base: &str, plural: &str, label_selector: Option<&str>) -> String {
    let base = base.trim_end_matches('/');
    match label_selector {
        Some(ls) => format!("{base}/api/v1/{plural}?labelSelector={ls}"),
        None => format!("{base}/api/v1/{plural}"),
    }
}

/// Build a single-object URL.
pub fn object_url(base: &str, plural: &str, name: &str) -> String {
    let base = base.trim_end_matches('/');
    format!("{base}/api/v1/{plural}/{name}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn plural_normalizes_singular_and_plural() {
        assert_eq!(plural_for("container"), Some("containers"));
        assert_eq!(plural_for("workers"), Some("workers"));
        assert_eq!(plural_for("lease"), Some("leases"));
        assert_eq!(plural_for("widget"), None);
    }

    #[test]
    fn server_precedence_flag_env_config_default() {
        let cfg = Config {
            server: Some("http://cfg:1".into()),
            token: None,
        };
        assert_eq!(
            resolve_server(Some("http://flag:1"), Some("http://env:1"), &cfg),
            "http://flag:1"
        );
        assert_eq!(
            resolve_server(None, Some("http://env:1"), &cfg),
            "http://env:1"
        );
        assert_eq!(resolve_server(None, None, &cfg), "http://cfg:1");
        assert_eq!(
            resolve_server(None, None, &Config::default()),
            DEFAULT_SERVER
        );
    }

    #[test]
    fn token_precedence_flag_env_config() {
        let cfg = Config {
            server: None,
            token: Some("cfgtok".into()),
        };
        assert_eq!(
            resolve_token(Some("flagtok"), Some("envtok"), &cfg).as_deref(),
            Some("flagtok")
        );
        assert_eq!(
            resolve_token(None, Some("envtok"), &cfg).as_deref(),
            Some("envtok")
        );
        assert_eq!(resolve_token(None, None, &cfg).as_deref(), Some("cfgtok"));
        assert_eq!(resolve_token(None, None, &Config::default()), None);
    }

    #[test]
    fn config_round_trips_through_disk() {
        // Isolate HOME so we don't touch the real ~/.velos.
        let tmp = std::env::temp_dir().join(format!("velosctl-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        // SAFETY: single-threaded test; restored by process teardown.
        unsafe { std::env::set_var("HOME", &tmp) };

        let cfg = Config {
            server: Some("http://h:9".into()),
            token: Some("tok".into()),
        };
        save_config(&cfg).unwrap();
        let back = load_config();
        assert_eq!(back.server.as_deref(), Some("http://h:9"));
        assert_eq!(back.token.as_deref(), Some("tok"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(config_path().unwrap())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn urls_trim_trailing_slash_and_add_selector() {
        assert_eq!(
            collection_url("http://h:8080/", "containers", None),
            "http://h:8080/api/v1/containers"
        );
        assert_eq!(
            collection_url("http://h:8080", "containers", Some("team=a")),
            "http://h:8080/api/v1/containers?labelSelector=team=a"
        );
        assert_eq!(
            object_url("http://h:8080/", "workers", "w1"),
            "http://h:8080/api/v1/workers/w1"
        );
    }
}
