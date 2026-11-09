//! Local CLI for smoke-testing WASM execution.
use anyhow::{Context, Result};
use clap::Parser;
use sandbox_common::{ExecutionRequest, Language, NetworkPolicy, ResourceLimits};
use sandbox_runtime::Runtime;
use std::{fs, path::PathBuf};

#[derive(Debug, Parser)]
#[command(
    name = "sandbox-cli",
    about = "Run a hex-encoded WebAssembly module in the sandbox runtime"
)]
struct Args {
    /// Hex-encoded WASM file.
    #[arg(short, long)]
    file: PathBuf,
    /// Optional timeout in milliseconds.
    #[arg(long, default_value_t = 5_000)]
    timeout_ms: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();
    let code =
        fs::read_to_string(&args.file).with_context(|| format!("read {}", args.file.display()))?;
    let request = ExecutionRequest {
        id: uuid::Uuid::new_v4(),
        language: Language::Wasm,
        code,
        limits: ResourceLimits {
            timeout_ms: args.timeout_ms,
            ..Default::default()
        },
        network: NetworkPolicy::Deny,
    };
    let result = Runtime::default().execute(request).await?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
