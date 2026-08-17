//! NeoBrowser — a fast, stealthy MCP browser-automation server that drives real
//! Chrome via CDP. `serve` runs the MCP server (default); `doctor` checks the
//! environment; `tools` prints the tool catalog for humans/AIs.

mod cli;

use cli::doctor::{doctor, doctor_json};
use cli::subcommands::{bridge_cmd, config_cmd, http_cmd, trace_cmd};
use cli::tools::print_tools;
use neobrowser::mcp;

const HELP: &str = "\
neobrowser — MCP browser automation over real Chrome

USAGE:
  neobrowser [serve]     Run the MCP server on stdio (default)
  neobrowser doctor      Check Chrome discovery + a live CDP smoke test
  neobrowser doctor --json   Same checks as machine-readable JSON (exit 1 if any fail)
  neobrowser config schema      Print the config file's JSON Schema
  neobrowser config init <safe|developer|autonomous|ci>
                                Write a starter neobrowser.toml
  neobrowser config show        Show the resolved config and where it came from
  neobrowser trace list         List recorded evidence bundles, newest first
  neobrowser trace open <id>    Print one bundle (redacted, shareable)
  neobrowser bridge token       Print the bridge token to paste into the extension
  neobrowser http token         Print the MCP HTTP transport bearer token
  neobrowser tools       Print the tool catalog as JSON
  neobrowser tools --markdown   Print the tool catalog as Markdown
  neobrowser --version   Print the version

Key env vars: NEOBROWSER_REAL_PROFILE, NEOBROWSER_ATTACH_PORT,
NEOBROWSER_CHROME_BIN, NEOBROWSER_HOME, NEOBROWSER_PROXY, ANTHROPIC_API_KEY.";

#[tokio::main]
async fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    // stderr, always: stdout is the MCP transport, and a log line written there
    // corrupts the protocol stream.
    if std::env::var("NEOBROWSER_LOG_FORMAT").as_deref() == Ok("json") {
        tracing_subscriber::fmt()
            .json()
            .flatten_event(true)
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .init();
    }

    // Config is applied BEFORE anything reads the environment, so a file-provided
    // policy or vault setting is in place by the time the server or doctor look.
    // A broken config is fatal rather than ignored: continuing with silently
    // different settings than the file asked for is the worse outcome.
    match neobrowser::config::load() {
        Ok(Some((path, cfg))) => {
            let applied = cfg.apply_to_env();
            tracing::info!(
                config = %path.display(),
                version = cfg.version,
                applied = ?applied,
                "loaded configuration"
            );
        }
        Ok(None) => {}
        Err(e) => {
            eprintln!("neobrowser: {e}");
            std::process::exit(2);
        }
    }

    let arg = std::env::args().nth(1).unwrap_or_default();
    match arg.as_str() {
        "doctor" => {
            if std::env::args().any(|a| a == "--json") {
                doctor_json().await
            } else {
                doctor().await
            }
        }
        "tools" => print_tools(),
        "config" => config_cmd(),
        "trace" => trace_cmd(),
        "bridge" => bridge_cmd(),
        "http" => http_cmd(),
        "--version" | "-v" => println!("{}", env!("CARGO_PKG_VERSION")),
        "--help" | "-h" => println!("{HELP}"),
        "" | "serve" => mcp::serve().await,
        other => {
            eprintln!("neobrowser: unknown command '{other}'.\n\n{HELP}");
            std::process::exit(2);
        }
    }
}
