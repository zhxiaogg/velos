use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde_json::Value;
use velosctl::{collection_url, object_url, plural_for};

/// A thin CLI over the Velos REST API.
#[derive(Parser, Debug)]
#[command(name = "velosctl", version)]
struct Cli {
    /// apiserver base URL.
    #[arg(long, default_value = "http://127.0.0.1:8080", global = true)]
    server: String,

    /// Bearer credential for authenticated apiservers.
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
        bail!("apiserver returned {status}: {text}");
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

    match cli.command {
        Command::Get {
            kind,
            name,
            selector,
        } => {
            let plural = plural_for(&kind).with_context(|| format!("unknown kind: {kind}"))?;
            let url = match &name {
                Some(n) => object_url(&cli.server, plural, n),
                None => collection_url(&cli.server, plural, selector.as_deref()),
            };
            let resp = client(&cli.token, http.get(url)).send().await?;
            print_json(&body_or_error(resp).await?)?;
        }
        Command::Delete { kind, name } => {
            let plural = plural_for(&kind).with_context(|| format!("unknown kind: {kind}"))?;
            let resp = client(
                &cli.token,
                http.delete(object_url(&cli.server, plural, &name)),
            )
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
            let resp = client(
                &cli.token,
                http.post(collection_url(&cli.server, plural, None)),
            )
            .json(&body)
            .send()
            .await?;
            print_json(&body_or_error(resp).await?)?;
        }
        Command::Token(TokenCommand::Create { ttl }) => {
            let url = format!("{}/auth/v1/tokens", cli.server.trim_end_matches('/'));
            let resp = client(&cli.token, http.post(url))
                .json(&serde_json::json!({ "ttlSeconds": ttl }))
                .send()
                .await?;
            print_json(&body_or_error(resp).await?)?;
        }
    }
    Ok(())
}
