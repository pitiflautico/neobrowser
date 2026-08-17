//! The remaining subcommands: `http`, `bridge`, `trace`, `config`.
//!
//! Each is small and each is a different transport or store, so they sit together rather
//! than each claiming a file.

//! NeoBrowser — a fast, stealthy MCP browser-automation server that drives real
//! Chrome via CDP. `serve` runs the MCP server (default); `doctor` checks the
//! environment; `tools` prints the tool catalog for humans/AIs.

/// `http token` — print the bearer token for the MCP HTTP transport.
pub fn http_cmd() {
    match std::env::args().nth(2).unwrap_or_default().as_str() {
        "token" => match neobrowser::http_transport::read_token_file() {
            Ok(t) if !t.is_empty() => println!("{t}"),
            _ => {
                eprintln!(
                    "no HTTP token yet. Start the server with NEOBROWSER_HTTP_PORT set, then run this again."
                );
                std::process::exit(2);
            }
        },
        other => {
            eprintln!("neobrowser http: unknown subcommand {other:?}. Use token.");
            std::process::exit(2);
        }
    }
}

/// `bridge token` — print the token the extension needs.
///
/// A manual copy/paste, deliberately: the extension cannot read a file, and any
/// automatic handover over the same loopback port would be readable by exactly the
/// attacker the token exists to stop.
pub fn bridge_cmd() {
    match std::env::args().nth(2).unwrap_or_default().as_str() {
        "token" => match neobrowser::bridge::read_token_file() {
            Ok(t) if !t.is_empty() => println!("{t}"),
            _ => {
                eprintln!(
                    "no bridge token yet. Start the server with NEOBROWSER_BRIDGE_PORT set, \
                     then run this again."
                );
                std::process::exit(2);
            }
        },
        other => {
            eprintln!("neobrowser bridge: unknown subcommand {other:?}. Use token.");
            std::process::exit(2);
        }
    }
}

/// `trace list | open <id>` — inspect a recorded run.
///
/// Bundles are redacted when written, so `open` prints something that can be attached
/// to a bug report without a second review pass.
pub fn trace_cmd() {
    use neobrowser::trace;
    match std::env::args().nth(2).unwrap_or_default().as_str() {
        "list" => {
            let ids = trace::list_bundles();
            if ids.is_empty() {
                println!("no traces recorded yet (they are written when a session exits)");
                return;
            }
            for id in ids {
                println!("{id}");
            }
        }
        "open" => {
            let Some(id) = std::env::args().nth(3) else {
                eprintln!("neobrowser trace open: needs a trace id (see `trace list`)");
                std::process::exit(2);
            };
            match trace::read_bundle(&id) {
                Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default()),
                Err(e) => {
                    eprintln!("neobrowser trace open {id}: {e}");
                    std::process::exit(2);
                }
            }
        }
        other => {
            eprintln!("neobrowser trace: unknown subcommand {other:?}. Use list | open <id>.");
            std::process::exit(2);
        }
    }
}

/// `config schema | init <profile> | show`.
pub fn config_cmd() {
    use neobrowser::config;
    let sub = std::env::args().nth(2).unwrap_or_default();
    match sub.as_str() {
        "schema" => println!(
            "{}",
            serde_json::to_string_pretty(&config::json_schema()).unwrap_or_default()
        ),
        "init" => {
            let name = std::env::args()
                .nth(3)
                .unwrap_or_else(|| "developer".into());
            let path = std::path::PathBuf::from("neobrowser.toml");
            match config::write_template(&path, &name) {
                Ok(()) => println!("wrote {} ({name} profile)", path.display()),
                Err(e) => {
                    eprintln!("neobrowser config init: {e}");
                    std::process::exit(2);
                }
            }
        }
        "show" => match config::load() {
            Ok(Some((path, cfg))) => {
                println!("config:  {}", path.display());
                println!("version: {}", cfg.version);
                for key in cfg.keys() {
                    println!("  {key} = {}", cfg.get(key).unwrap_or(""));
                }
            }
            Ok(None) => {
                println!("no config file found. Searched:");
                for p in config::candidate_paths() {
                    println!("  {}", p.display());
                }
            }
            Err(e) => {
                eprintln!("neobrowser config show: {e}");
                std::process::exit(2);
            }
        },
        other => {
            eprintln!("neobrowser config: unknown subcommand {other:?}. Use schema | init | show.");
            std::process::exit(2);
        }
    }
}
