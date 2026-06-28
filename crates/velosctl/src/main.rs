use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde_json::Value;
use velosctl::{collection_url, object_url, plural_for};

/// A thin CLI over the Velos REST API.
#[derive(Parser, Debug)]
#[command(name = "velosctl", version)]
struct Cli {
    /// server base URL (overrides VELOS_SERVER and the saved config).
    #[arg(long, global = true)]
    server: Option<String>,

    /// Bearer credential (overrides VELOS_TOKEN and the saved config).
    #[arg(long, global = true)]
    token: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// List a resource kind, or get one by name.
    Get {
        kind: String,
        name: Option<String>,
        /// Filter a list by label selector, e.g. team=a.
        #[arg(long)]
        selector: Option<String>,
    },
    /// Delete a resource by name.
    Delete { kind: String, name: String },
    /// Create a resource from a JSON file.
    Apply {
        kind: String,
        #[arg(short, long)]
        file: String,
    },
    /// Bootstrap-token operations.
    #[command(subcommand)]
    Token(TokenCommand),
    /// Validate a token against the server and save it to ~/.velos/config.
    Login {
        #[arg(long)]
        token: String,
    },
    /// Remove the saved credential.
    Logout,
}

#[derive(Subcommand, Debug)]
enum TokenCommand {
    /// Mint a bootstrap token.
    Create {
        #[arg(long, default_value_t = 86400)]
        ttl: i64,
    },
}

fn client(token: &Option<String>, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    match token {
        Some(t) => rb.bearer_auth(t),
        None => rb,
    }
}

async fn body_or_error(resp: reqwest::Response) -> Result<Value> {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("server returned {status}: {text}");
    }
    if text.is_empty() {
        return Ok(Value::Null);
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

fn print_json(v: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let http = reqwest::Client::new();

    // Resolve server + token: flag > env > saved config (server also > default).
    let cfg = velosctl::load_config();
    let server = velosctl::resolve_server(
        cli.server.as_deref(),
        std::env::var("VELOS_SERVER").ok().as_deref(),
        &cfg,
    );
    let token = velosctl::resolve_token(
        cli.token.as_deref(),
        std::env::var("VELOS_TOKEN").ok().as_deref(),
        &cfg,
    );

    match cli.command {
        Command::Get {
            kind,
            name,
            selector,
        } => {
            let plural = plural_for(&kind).with_context(|| format!("unknown kind: {kind}"))?;
            let url = match &name {
                Some(n) => object_url(&server, plural, n),
                None => collection_url(&server, plural, selector.as_deref()),
            };
            let resp = client(&token, http.get(url)).send().await?;
            print_json(&body_or_error(resp).await?)?;
        }
        Command::Delete { kind, name } => {
            let plural = plural_for(&kind).with_context(|| format!("unknown kind: {kind}"))?;
            let resp = client(&token, http.delete(object_url(&server, plural, &name)))
                .send()
                .await?;
            let status = resp.status();
            if !status.is_success() {
                bail!("delete failed: {status}");
            }
            println!("{kind}/{name} deleted");
        }
        Command::Apply { kind, file } => {
            let plural = plural_for(&kind).with_context(|| format!("unknown kind: {kind}"))?;
            let contents =
                std::fs::read_to_string(&file).with_context(|| format!("reading {file}"))?;
            let body: Value = serde_json::from_str(&contents).context("parsing JSON file")?;
            let resp = client(&token, http.post(collection_url(&server, plural, None)))
                .json(&body)
                .send()
                .await?;
            print_json(&body_or_error(resp).await?)?;
        }
        Command::Token(TokenCommand::Create { ttl }) => {
            let url = format!("{}/auth/v1/tokens", server.trim_end_matches('/'));
            let resp = client(&token, http.post(url))
                .json(&serde_json::json!({ "ttlSeconds": ttl }))
                .send()
                .await?;
            print_json(&body_or_error(resp).await?)?;
        }
        Command::Login { token } => {
            // Validate the token against the resolved server before saving.
            let url = format!("{}/auth/v1/me", server.trim_end_matches('/'));
            let resp = http.get(url).bearer_auth(&token).send().await?;
            if !resp.status().is_success() {
                bail!("token rejected by {server}: {}", resp.status());
            }
            let saved = velosctl::Config {
                server: Some(server.clone()),
                token: Some(token),
            };
            velosctl::save_config(&saved)?;
            println!("logged in to {server}");
        }
        Command::Logout => {
            velosctl::save_config(&velosctl::Config::default())?;
            println!("logged out");
        }
    }
    Ok(())
}
