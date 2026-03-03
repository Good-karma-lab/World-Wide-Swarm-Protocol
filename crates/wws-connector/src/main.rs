//! CLI binary entry point for the WWS.Connector sidecar.
//!
//! Usage:
//!   wws-connector [OPTIONS]
//!
//! Options:
//!   -c, --config <FILE>    Path to configuration TOML file
//!   -l, --listen <ADDR>    P2P listen address (overrides config)
//!   -r, --rpc <ADDR>       RPC bind address (overrides config)
//!   -b, --bootstrap <ADDR> Bootstrap peer multiaddress (repeatable)
//!   -v, --verbose          Increase logging verbosity
//!   --agent-name <NAME>    Set the agent name
//!   --tui                  Launch the TUI monitoring dashboard
//!   --console              Launch the operator console (interactive task injection + hierarchy)

use std::path::PathBuf;

use clap::Parser;

use wws_connector::config::ConnectorConfig;
use wws_connector::connector::WwsConnector;
use wws_connector::file_server::FileServer;
use wws_connector::rpc_server::RpcServer;

/// WWS.Connector - Sidecar process connecting AI agents to the swarm.
#[derive(Parser, Debug)]
#[command(name = "wws-connector")]
#[command(about = "WWS.Connector sidecar for AI agent swarm participation")]
#[command(version)]
struct Cli {
    /// Path to configuration TOML file.
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// P2P listen address (e.g., /ip4/0.0.0.0/tcp/9000).
    #[arg(short, long, value_name = "MULTIADDR")]
    listen: Option<String>,

    /// JSON-RPC server bind address (e.g., 127.0.0.1:9370).
    #[arg(short, long, value_name = "ADDR")]
    rpc: Option<String>,

    /// Bootstrap peer multiaddress (can be specified multiple times).
    #[arg(short, long, value_name = "MULTIADDR")]
    bootstrap: Vec<String>,

    /// Increase logging verbosity (can be repeated: -v, -vv, -vvv).
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Set the agent name.
    #[arg(long, value_name = "NAME")]
    agent_name: Option<String>,

    /// Swarm ID to join (default: "public" for the open public swarm).
    #[arg(long, value_name = "SWARM_ID")]
    swarm_id: Option<String>,

    /// Authentication token for joining a private swarm.
    #[arg(long, value_name = "TOKEN")]
    swarm_token: Option<String>,

    /// Create a new private swarm with this name instead of joining an existing one.
    #[arg(long, value_name = "NAME")]
    create_swarm: Option<String>,

    /// Launch the terminal UI dashboard for live monitoring.
    #[arg(long)]
    tui: bool,

    /// Launch the operator console for interactive task injection and hierarchy view.
    #[arg(long)]
    console: bool,

    /// HTTP file server bind address for serving agent onboarding docs.
    #[arg(long, value_name = "ADDR")]
    files_addr: Option<String>,

    /// Disable the HTTP file server.
    #[arg(long)]
    no_files: bool,

    /// Path to Ed25519 key file (default: ~/.config/wws-connector/<name>.key).
    #[arg(long, value_name = "PATH")]
    key_file: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Load configuration.
    let mut config = ConnectorConfig::load(cli.config.as_deref())?;

    // Apply CLI overrides.
    if let Some(listen) = cli.listen {
        config.network.listen_addr = listen;
    }
    if let Some(rpc) = cli.rpc {
        config.rpc.bind_addr = rpc;
    }
    if !cli.bootstrap.is_empty() {
        config.network.bootstrap_peers = cli.bootstrap;
    }
    if let Some(name) = cli.agent_name {
        config.agent.name = name;
    }
    if let Some(swarm_id) = cli.swarm_id {
        config.swarm.swarm_id = swarm_id;
    }
    if let Some(token) = cli.swarm_token {
        config.swarm.token = Some(token);
    }
    if let Some(name) = cli.create_swarm {
        // When creating a new swarm, generate a new swarm ID and mark it as private.
        config.swarm.swarm_id = uuid::Uuid::new_v4().to_string();
        config.swarm.name = name;
        config.swarm.is_public = false;
    }
    if let Some(addr) = cli.files_addr {
        config.file_server.bind_addr = addr;
    }
    if cli.no_files {
        config.file_server.enabled = false;
    }

    // Adjust log level based on verbosity.
    let log_level = match cli.verbose {
        0 => &config.logging.level,
        1 => "debug",
        2 => "trace",
        _ => "trace",
    };

    // Initialize logging.
    // When TUI/console mode is enabled, redirect logs to a file.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level));

    if cli.tui || cli.console {
        // In TUI/console mode, write logs to a file instead of stdout/stderr.
        let log_dir = std::env::temp_dir().join("wws-logs");
        std::fs::create_dir_all(&log_dir)?;
        let log_file = log_dir.join(format!("{}.log", config.agent.name));
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file)?;

        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(true)
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(file))
            .init();

        eprintln!("Logs: {}", log_file.display());
        eprintln!("  tail -f {}", log_file.display());
        eprintln!();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(true)
            .init();
    }

    tracing::info!(
        agent = %config.agent.name,
        listen = %config.network.listen_addr,
        rpc = %config.rpc.bind_addr,
        swarm_id = %config.swarm.swarm_id,
        swarm_name = %config.swarm.name,
        swarm_public = config.swarm.is_public,
        files_server = %config.file_server.bind_addr,
        files_enabled = config.file_server.enabled,
        "Starting WWS.Connector"
    );

    // Load or generate persistent identity key
    let key_path = cli.key_file.unwrap_or_else(|| {
        wws_connector::identity_store::default_key_path(&config.agent.name)
    });
    let _signing_key = wws_connector::identity_store::load_or_generate_key(&key_path)?;
    tracing::info!(key_path = %key_path.display(), "Identity key loaded");

    // Create the connector.
    let connector = WwsConnector::new(config.clone())?;

    // Get handles for the RPC server.
    let state = connector.shared_state();
    let network_handle = connector.network_handle();

    // Start the RPC server in a background task.
    let rpc_server = RpcServer::new(
        config.rpc.bind_addr.clone(),
        state.clone(),
        network_handle,
        config.rpc.max_connections,
    );

    tokio::spawn(async move {
        if let Err(e) = rpc_server.run().await {
            tracing::error!(error = %e, "RPC server error");
        }
    });

    // Start the HTTP file server if enabled.
    if config.file_server.enabled {
        let file_server = FileServer::new(
            config.file_server.bind_addr.clone(),
            state.clone(),
            connector.network_handle(),
        );
        tokio::spawn(async move {
            if let Err(e) = file_server.run().await {
                tracing::error!(error = %e, "HTTP file server error");
            }
        });
    }

    if cli.console {
        // Launch the operator console.
        let console_state = state.clone();
        let console_network_handle = connector.network_handle();
        let console_handle = tokio::spawn(async move {
            if let Err(e) =
                wws_connector::operator_console::run_operator_console(
                    console_state,
                    console_network_handle,
                )
                .await
            {
                let err_msg = e.to_string();
                if err_msg.contains("TTY") || err_msg.contains("terminal") {
                    tracing::warn!(
                        "Console mode disabled: {}. Continuing in headless mode.",
                        err_msg
                    );
                } else {
                    tracing::error!(error = %e, "Operator console error");
                }
            }
        });

        tokio::select! {
            result = connector.run() => {
                result?;
            }
            _ = console_handle => {
                // Console exited, shutting down.
            }
        }
    } else if cli.tui {
        // Spawn the TUI in a separate task.
        let tui_state = state.clone();
        let tui_handle = tokio::spawn(async move {
            if let Err(e) = wws_connector::tui::run_tui(tui_state).await {
                let err_msg = e.to_string();
                if err_msg.contains("TTY") || err_msg.contains("terminal") {
                    tracing::warn!(
                        "TUI mode disabled: {}. Continuing in non-TUI mode.",
                        err_msg
                    );
                } else {
                    tracing::error!(error = %e, "TUI error");
                }
            }
        });

        tokio::select! {
            result = connector.run() => {
                result?;
            }
            _ = tui_handle => {
                // TUI exited (user pressed 'q'), shutting down.
            }
        }
    } else {
        // Run the connector (this blocks until shutdown).
        connector.run().await?;
    }

    Ok(())
}
