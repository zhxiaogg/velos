//! Self-installation as a macOS launchd LaunchAgent.
//!
//! macOS "Local Network Privacy" silently blocks a bare LaunchAgent from
//! reaching a LAN server: a launchd job has no GUI-app ancestor for the system
//! to attribute (and prompt) the connection to, so it is denied with no UI.
//! The fix (Apple TN3179) is to give the worker a real *app-bundle identity*:
//! wrap the binary in a code-signed `.app` carrying a bundle id and an
//! `NSLocalNetworkUsageDescription`, then point the LaunchAgent at it via
//! `AssociatedBundleIdentifiers`. With that in place macOS shows the native
//! "… wants to access your local network" prompt on the first connection.
//!
//! This module holds only the *pure* rendering of those files and the persisted
//! config type; `main.rs` performs the filesystem + `launchctl` side effects.

use serde::{Deserialize, Serialize};

/// The launchd label and app-bundle identifier for the worker daemon.
pub const BUNDLE_ID: &str = "com.velos.veloslet";
/// Human-facing bundle name shown in the Local Network privacy prompt/list.
pub const BUNDLE_DISPLAY_NAME: &str = "Velos Worker";
/// The executable name inside the app bundle (`Velos.app/Contents/MacOS/<name>`).
pub const BUNDLE_EXECUTABLE: &str = "veloslet";

fn default_reconcile_secs() -> u64 {
    5
}
fn default_heartbeat_secs() -> u64 {
    10
}
fn default_lease_secs() -> u32 {
    40
}

/// Persisted worker configuration (written as JSON to `~/.velos/veloslet.json`).
///
/// The bootstrap token lives here — not in the LaunchAgent's argument vector —
/// so it never shows up in the process table (`ps`). The file is created `0600`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerConfig {
    /// Server base URL, e.g. `http://192.168.68.60:8088`.
    pub server: String,
    /// This worker's name.
    pub node: String,
    /// Bootstrap token (`id.secret`) used to register on each start.
    pub token: String,
    #[serde(default = "default_reconcile_secs")]
    pub reconcile_secs: u64,
    #[serde(default = "default_heartbeat_secs")]
    pub heartbeat_secs: u64,
    #[serde(default = "default_lease_secs")]
    pub lease_secs: u32,
}

/// Minimal XML text escaping for plist `<string>` values.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Render the app-bundle `Info.plist` that gives the worker a stable identity
/// plus the local-network usage string macOS shows the user.
pub fn render_info_plist(version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Velos</string>
    <key>CFBundleDisplayName</key><string>{display}</string>
    <key>CFBundleIdentifier</key><string>{bundle_id}</string>
    <key>CFBundleExecutable</key><string>{exe}</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleVersion</key><string>{version}</string>
    <key>CFBundleShortVersionString</key><string>{version}</string>
    <key>LSBackgroundOnly</key><true/>
    <key>NSLocalNetworkUsageDescription</key>
    <string>Velos Worker connects to the Velos control-plane server on your local network to register this machine and reconcile containers.</string>
</dict>
</plist>
"#,
        display = xml_escape(BUNDLE_DISPLAY_NAME),
        bundle_id = xml_escape(BUNDLE_ID),
        exe = xml_escape(BUNDLE_EXECUTABLE),
        version = xml_escape(version),
    )
}

/// Render the LaunchAgent plist. `AssociatedBundleIdentifiers` is what lets
/// Local Network Privacy attribute the agent's traffic to the signed bundle.
pub fn render_launch_agent(
    program_args: &[String],
    path_env: &str,
    stdout_path: &str,
    stderr_path: &str,
) -> String {
    let args_xml = program_args
        .iter()
        .map(|a| format!("        <string>{}</string>", xml_escape(a)))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>

    <!-- Tell macOS which signed bundle is responsible for this agent's network
         access so Local Network Privacy can attribute (and prompt for) the
         connection instead of silently denying it. (Apple TN3179) -->
    <key>AssociatedBundleIdentifiers</key>
    <array>
        <string>{label}</string>
    </array>

    <key>ProgramArguments</key>
    <array>
{args_xml}
    </array>

    <!-- launchd's default PATH is minimal; add /usr/local/bin so the Apple
         `container` CLI is discoverable by the runtime. -->
    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>{path_env}</string>
        <key>RUST_LOG</key>
        <string>info</string>
    </dict>

    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>

    <key>StandardOutPath</key>
    <string>{stdout}</string>
    <key>StandardErrorPath</key>
    <string>{stderr}</string>
</dict>
</plist>
"#,
        label = xml_escape(BUNDLE_ID),
        args_xml = args_xml,
        path_env = xml_escape(path_env),
        stdout = xml_escape(stdout_path),
        stderr = xml_escape(stderr_path),
    )
}

#[cfg(test)]
#[cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrips_through_json() {
        let cfg = WorkerConfig {
            server: "http://192.168.68.60:8088".to_string(),
            node: "node-a".to_string(),
            token: "id.secret".to_string(),
            reconcile_secs: 5,
            heartbeat_secs: 10,
            lease_secs: 40,
        };
        let text = serde_json::to_string(&cfg).unwrap();
        let back: WorkerConfig = serde_json::from_str(&text).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn config_applies_interval_defaults_when_omitted() {
        let cfg: WorkerConfig =
            serde_json::from_str(r#"{"server":"http://h:1","node":"n","token":"t"}"#).unwrap();
        assert_eq!(cfg.reconcile_secs, 5);
        assert_eq!(cfg.heartbeat_secs, 10);
        assert_eq!(cfg.lease_secs, 40);
    }

    #[test]
    fn launch_agent_carries_associated_bundle_id_and_args() {
        let args = vec![
            "/Applications/Velos.app/Contents/MacOS/veloslet".to_string(),
            "run".to_string(),
            "--config".to_string(),
            "/home/u/.velos/veloslet.json".to_string(),
        ];
        let plist = render_launch_agent(&args, "/usr/local/bin:/usr/bin:/bin", "/o.log", "/e.log");
        assert!(plist.contains("<key>AssociatedBundleIdentifiers</key>"));
        assert!(plist.contains("<string>com.velos.veloslet</string>"));
        assert!(plist.contains("<string>run</string>"));
        assert!(plist.contains("<string>/home/u/.velos/veloslet.json</string>"));
    }

    #[test]
    fn info_plist_declares_identity_and_local_network_usage() {
        let info = render_info_plist("0.1.1");
        assert!(info.contains("<key>CFBundleIdentifier</key><string>com.velos.veloslet</string>"));
        assert!(info.contains("<key>NSLocalNetworkUsageDescription</key>"));
    }

    #[test]
    fn xml_escape_neutralizes_markup() {
        assert_eq!(xml_escape("a&b<c>"), "a&amp;b&lt;c&gt;");
    }
}
