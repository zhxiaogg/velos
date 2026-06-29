use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use velos_runtime::{AppleContainer, ContainerRuntime};
use veloslet::daemon::{self, BUNDLE_EXECUTABLE, BUNDLE_ID, WorkerConfig};
use veloslet::host::{detect_host, validate_capacity};
use veloslet::memory::Memory;
use veloslet::{ApiClient, run_loop};

mod signing;

/// The Velos worker daemon.
#[derive(Parser, Debug)]
#[command(name = "veloslet", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the worker loop in the foreground (this is what the LaunchAgent
    /// invokes). Reads `--config` and/or the individual flags.
    Run(RunArgs),
    /// Install and start the worker as a macOS launchd LaunchAgent: wraps the
    /// binary in a signed app bundle (for Local Network privacy), writes the
    /// config, and loads the agent.
    Install(InstallArgs),
    /// Stop and remove the LaunchAgent.
    Uninstall(UninstallArgs),
}

#[derive(clap::Args, Debug)]
struct RunArgs {
    /// Path to a JSON config file (`~/.velos/veloslet.json` by convention).
    #[arg(long)]
    config: Option<PathBuf>,
    /// server base URL, e.g. http://127.0.0.1:8080 (overrides config).
    #[arg(long)]
    server: Option<String>,
    /// This worker's name (overrides config).
    #[arg(long)]
    node: Option<String>,
    /// Bootstrap token (`id.secret`) used to register on first start.
    #[arg(long)]
    token: Option<String>,
    /// Advertised CPU cores (overrides config).
    #[arg(long)]
    cpu: Option<u32>,
    /// Advertised memory, e.g. `8G` (overrides config).
    #[arg(long)]
    memory: Option<Memory>,
    /// Reconcile interval in seconds.
    #[arg(long)]
    reconcile_secs: Option<u64>,
    /// Heartbeat (lease renew) interval in seconds.
    #[arg(long)]
    heartbeat_secs: Option<u64>,
    /// Lease duration in seconds.
    #[arg(long)]
    lease_secs: Option<u32>,
}

#[derive(clap::Args, Debug)]
struct InstallArgs {
    /// server base URL, e.g. http://192.168.68.60:8088
    #[arg(long)]
    server: String,
    /// This worker's name.
    #[arg(long)]
    node: String,
    /// Bootstrap token (`id.secret`), e.g. from `velosctl token create`.
    #[arg(long)]
    token: String,
    /// Advertised CPU cores.
    #[arg(long)]
    cpu: u32,
    /// Advertised memory, e.g. `8G`.
    #[arg(long)]
    memory: Memory,
    /// Reconcile interval in seconds.
    #[arg(long, default_value_t = 5)]
    reconcile_secs: u64,
    /// Heartbeat (lease renew) interval in seconds.
    #[arg(long, default_value_t = 10)]
    heartbeat_secs: u64,
    /// Lease duration in seconds.
    #[arg(long, default_value_t = 40)]
    lease_secs: u32,
}

#[derive(clap::Args, Debug)]
struct UninstallArgs {
    /// Also delete the app bundle and the saved config file.
    #[arg(long)]
    purge: bool,
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// The on-disk locations the daemon owns, all under the user's home directory.
struct Paths {
    bundle_dir: PathBuf,
    bundle_bin: PathBuf,
    info_plist: PathBuf,
    config_file: PathBuf,
    /// Persistent self-signed signing identity (cert + key). Survives uninstall
    /// so the bundle's code-signature stays stable across reinstalls.
    codesign_dir: PathBuf,
    agent_plist: PathBuf,
    stdout_log: PathBuf,
    stderr_log: PathBuf,
}

impl Paths {
    fn resolve() -> Result<Self> {
        let home = dirs::home_dir().context("could not determine home directory")?;
        let bundle_dir = home.join("Applications/Velos.app");
        Ok(Self {
            bundle_bin: bundle_dir.join("Contents/MacOS").join(BUNDLE_EXECUTABLE),
            info_plist: bundle_dir.join("Contents/Info.plist"),
            bundle_dir,
            config_file: home.join(".velos/veloslet.json"),
            codesign_dir: home.join(".velos/codesign"),
            agent_plist: home
                .join("Library/LaunchAgents")
                .join(format!("{BUNDLE_ID}.plist")),
            stdout_log: home.join("Library/Logs/veloslet.out.log"),
            stderr_log: home.join("Library/Logs/veloslet.err.log"),
        })
    }
}

fn path_str(p: &Path) -> Result<&str> {
    p.to_str()
        .with_context(|| format!("path is not valid UTF-8: {}", p.display()))
}

// ---------------------------------------------------------------------------
// run
// ---------------------------------------------------------------------------

fn resolve_run_config(args: RunArgs) -> Result<WorkerConfig> {
    // Start from the config file if given, else an all-flags base.
    let mut cfg = match &args.config {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading config {}", path.display()))?;
            serde_json::from_str::<WorkerConfig>(&text)
                .with_context(|| format!("parsing config {}", path.display()))?
        }
        None => WorkerConfig {
            server: args
                .server
                .clone()
                .context("--server is required when --config is not given")?,
            node: args
                .node
                .clone()
                .context("--node is required when --config is not given")?,
            token: args
                .token
                .clone()
                .context("--token is required when --config is not given")?,
            cpu: args
                .cpu
                .context("--cpu is required when --config is not given")?,
            memory: args
                .memory
                .context("--memory is required when --config is not given")?,
            reconcile_secs: 5,
            heartbeat_secs: 10,
            lease_secs: 40,
        },
    };
    // Explicit flags override whatever the file provided.
    if let Some(v) = args.server {
        cfg.server = v;
    }
    if let Some(v) = args.node {
        cfg.node = v;
    }
    if let Some(v) = args.token {
        cfg.token = v;
    }
    if let Some(v) = args.cpu {
        cfg.cpu = v;
    }
    if let Some(v) = args.memory {
        cfg.memory = v;
    }
    if let Some(v) = args.reconcile_secs {
        cfg.reconcile_secs = v;
    }
    if let Some(v) = args.heartbeat_secs {
        cfg.heartbeat_secs = v;
    }
    if let Some(v) = args.lease_secs {
        cfg.lease_secs = v;
    }
    Ok(cfg)
}

async fn run(cfg: WorkerConfig) -> Result<()> {
    // Fail closed: never advertise more than the machine physically has.
    let host = detect_host()?;
    validate_capacity(cfg.cpu, cfg.memory, host)?;

    let runtime = AppleContainer::new();
    let runtime_version = runtime
        .version()
        .await
        .unwrap_or_else(|_| "unknown".to_string());

    // Register with the bootstrap token to obtain a worker credential.
    let boot = ApiClient::new(&cfg.server, Some(cfg.token.clone()));
    let request = serde_json::json!({
        "name": cfg.node,
        "capacity": { "cpu": cfg.cpu, "memoryBytes": cfg.memory.bytes() },
        "addresses": [],
        "containerRuntimeVersion": runtime_version,
    });
    // Retry registration in-process instead of exiting on failure. This keeps a
    // long-lived process alive so macOS can attribute (and the user can approve)
    // the Local Network privacy prompt — a process that exits immediately on the
    // first blocked connection tears the prompt's owner down before it can be
    // approved. It also rides out transient server outages.
    let resp = loop {
        match boot.register(&request).await {
            Ok(resp) => break resp,
            Err(e) => {
                tracing::warn!("register failed, retrying in 10s: {e}");
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        }
    };
    let credential = resp
        .get("token")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    tracing::info!("registered worker {}", cfg.node);

    let client = ApiClient::new(&cfg.server, credential.or(Some(cfg.token.clone())));
    let runtime: Arc<dyn ContainerRuntime> = Arc::new(runtime);

    tracing::info!("veloslet {} reconciling against {}", cfg.node, cfg.server);
    run_loop(
        client,
        runtime,
        cfg.node,
        Duration::from_secs(cfg.reconcile_secs),
        Duration::from_secs(cfg.heartbeat_secs),
        cfg.lease_secs,
    )
    .await;
    Ok(())
}

// ---------------------------------------------------------------------------
// install / uninstall (side effects)
// ---------------------------------------------------------------------------

fn write_file(path: &Path, contents: &str, mode: u32) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .with_context(|| format!("chmod {mode:o} {}", path.display()))?;
    Ok(())
}

/// Run `launchctl` quietly — we report success/failure ourselves, so suppress
/// its own output (notably the harmless "Unload failed" when nothing is loaded).
fn launchctl(args: &[&str]) -> Result<bool> {
    let status = Process::new("launchctl")
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("running launchctl")?;
    Ok(status.success())
}

fn install(args: InstallArgs) -> Result<()> {
    let paths = Paths::resolve()?;
    let version = env!("CARGO_PKG_VERSION");

    // Reject impossible capacity before touching the filesystem or launchd.
    let host = detect_host()?;
    validate_capacity(args.cpu, args.memory, host)?;

    // 1. App bundle: copy this running binary into Velos.app and give it a
    //    bundle identity so Local Network privacy can attribute its traffic.
    let src_exe = std::env::current_exe().context("locating the running veloslet binary")?;
    if let Some(parent) = paths.bundle_bin.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::copy(&src_exe, &paths.bundle_bin).with_context(|| {
        format!(
            "copying {} -> {}",
            src_exe.display(),
            paths.bundle_bin.display()
        )
    })?;
    std::fs::set_permissions(&paths.bundle_bin, std::fs::Permissions::from_mode(0o755))?;
    write_file(
        &paths.info_plist,
        &daemon::render_info_plist(version),
        0o644,
    )?;

    // 2. Code-sign the bundle with a *persistent* self-signed identity so macOS
    //    keeps the Local Network privacy grant across reinstalls. Ad-hoc signing
    //    would re-pin the grant to the cdhash and break it on every rebuild.
    let identity = signing::ensure_identity(&paths.codesign_dir)?;
    signing::sign_bundle(&paths.bundle_dir, BUNDLE_ID, identity)?;
    // Verify with the system codesign so a bad signature fails install loudly.
    let bundle = path_str(&paths.bundle_dir)?;
    let verified = Process::new("codesign")
        .args(["--verify", "--strict", bundle])
        .status()
        .context("running codesign --verify")?;
    if !verified.success() {
        bail!("codesign verification failed for {bundle}");
    }

    // 3. Persist config (0600 — it holds the bootstrap token).
    let cfg = WorkerConfig {
        server: args.server,
        node: args.node,
        token: args.token,
        cpu: args.cpu,
        memory: args.memory,
        reconcile_secs: args.reconcile_secs,
        heartbeat_secs: args.heartbeat_secs,
        lease_secs: args.lease_secs,
    };
    let cfg_json = serde_json::to_string_pretty(&cfg).context("serializing config")?;
    write_file(&paths.config_file, &cfg_json, 0o600)?;

    // 4. LaunchAgent plist pointing at the bundled binary + config.
    let program_args = vec![
        path_str(&paths.bundle_bin)?.to_string(),
        "run".to_string(),
        "--config".to_string(),
        path_str(&paths.config_file)?.to_string(),
    ];
    let agent = daemon::render_launch_agent(
        &program_args,
        "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
        path_str(&paths.stdout_log)?,
        path_str(&paths.stderr_log)?,
    );
    write_file(&paths.agent_plist, &agent, 0o644)?;

    // 5. (Re)load the agent.
    let agent_path = path_str(&paths.agent_plist)?;
    let _ = launchctl(&["unload", agent_path]);
    if !launchctl(&["load", "-w", agent_path])? {
        bail!("launchctl load failed for {agent_path}");
    }

    // Best-effort: surface the Local Network privacy pane in case the prompt is
    // missed. Approving the "{name} wants to access your local network" prompt
    // is the one manual step.
    let _ = Process::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_LocalNetwork")
        .status();

    println!("installed and started LaunchAgent {BUNDLE_ID}");
    println!("  bundle:  {}", paths.bundle_dir.display());
    println!("  config:  {}", paths.config_file.display());
    println!("  agent:   {}", paths.agent_plist.display());
    println!("  logs:    {}", paths.stdout_log.display());
    let name = daemon::BUNDLE_DISPLAY_NAME;
    println!(
        "\nApprove the macOS \"{name} wants to access your local network\" prompt\n\
         when it appears (or enable {name} under System Settings → Privacy &\n\
         Security → Local Network) — until then the worker cannot reach the server."
    );
    Ok(())
}

fn uninstall(args: UninstallArgs) -> Result<()> {
    let paths = Paths::resolve()?;
    if let Some(agent_path) = paths.agent_plist.to_str() {
        let _ = launchctl(&["unload", agent_path]);
    }
    remove_if_exists(&paths.agent_plist)?;
    if args.purge {
        remove_dir_if_exists(&paths.bundle_dir)?;
        remove_if_exists(&paths.config_file)?;
        println!("uninstalled LaunchAgent {BUNDLE_ID} and purged bundle + config");
    } else {
        println!(
            "uninstalled LaunchAgent {BUNDLE_ID} (kept bundle + config; pass --purge to remove)"
        );
    }
    println!(
        "Note: the Local Network privacy grant for {} remains in\n\
         System Settings → Privacy & Security → Local Network; remove it there if desired.",
        daemon::BUNDLE_DISPLAY_NAME
    );
    Ok(())
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

fn remove_dir_if_exists(path: &Path) -> Result<()> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    match Cli::parse().command {
        Command::Run(args) => run(resolve_run_config(args)?).await,
        Command::Install(args) => install(args),
        Command::Uninstall(args) => uninstall(args),
    }
}
