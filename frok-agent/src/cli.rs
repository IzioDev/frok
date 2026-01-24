use std::io::IsTerminal;

use clap::{Args, Parser, Subcommand};

use frok_protocol::IngressMode;

#[derive(Args, Debug, Clone, Default)]
pub(crate) struct RunOptions {
    /// Run without the TUI (daemon mode)
    #[arg(
        long,
        env = "FROK_HEADLESS",
        default_value_t = false,
        alias = "daemon",
        alias = "no-tui"
    )]
    pub(crate) headless: bool,

    /// Force the TUI even when stdout is not a TTY
    #[arg(
        long,
        env = "FROK_TUI",
        default_value_t = false,
        conflicts_with = "headless"
    )]
    pub(crate) tui: bool,

    /// Enable file logging in headless mode
    #[arg(long, env = "FROK_LOG_FILE", default_value_t = false)]
    pub(crate) log_file: bool,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy)]
enum QuickHttpMode {
    Http,
    Http2,
}

#[derive(Args, Debug, Clone)]
struct QuickOptions {
    /// Local port to expose
    port: u16,

    /// Route name/prefix (defaults to project name or "app")
    #[arg(long)]
    name: Option<String>,

    /// Create the route, print the URL, then exit
    #[arg(long, default_value_t = false)]
    once: bool,

    /// HTTP mode (http or http2)
    #[arg(long, value_enum, default_value = "http")]
    mode: QuickHttpMode,
}

#[derive(Args, Debug, Clone)]
struct QuickTcpOptions {
    /// Local port to expose
    port: u16,

    /// Route name/prefix (defaults to project name or "app")
    #[arg(long)]
    name: Option<String>,

    /// Create the route, print the URL, then exit
    #[arg(long, default_value_t = false)]
    once: bool,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run the agent (default)
    Run(RunOptions),

    /// Expose a local HTTP service
    Http(QuickOptions),

    /// Expose a local TCP service
    Tcp(QuickTcpOptions),
}

#[derive(Parser, Debug)]
#[command(
    name = "frok",
    version,
    about = "Frok agent - https://github.com/IzioDev/frok"
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[command(flatten)]
    run: RunOptions,

    /// Quick HTTP shortcut: frok <port>
    port: Option<u16>,

    /// Route name/prefix for quick mode
    #[arg(long)]
    name: Option<String>,

    /// Create the route, print the URL, then exit (quick mode)
    #[arg(long, default_value_t = false)]
    once: bool,

    /// HTTP mode for quick mode (http or http2)
    #[arg(long, value_enum, default_value = "http")]
    mode: QuickHttpMode,

    /// Edge endpoint (host or host:port)
    #[arg(
        long,
        env = "FROK_EDGE",
        default_value = "edge.frok.it:443",
        global = true
    )]
    pub(crate) edge: String,

    /// Public domain used for route hostnames (defaults to edge host root)
    #[arg(long, env = "FROK_PUBLIC_DOMAIN", global = true)]
    pub(crate) public_domain: Option<String>,

    /// Agent label (used for auth and hello)
    #[arg(
        long,
        env = "FROK_AGENT_LABEL",
        default_value = "agent-1",
        global = true
    )]
    pub(crate) agent_label: String,

    /// OIDC authorization code or full redirect URL (manual login)
    #[arg(long, global = true)]
    pub(crate) oidc_code: Option<String>,
}

impl Cli {
    pub(crate) fn run_options(&self) -> RunOptions {
        match &self.command {
            Some(Commands::Run(options)) => options.clone(),
            None => self.run.clone(),
            Some(_) => RunOptions::default(),
        }
    }

    pub(crate) fn launch_mode(&self) -> LaunchMode {
        match &self.command {
            Some(Commands::Run(_)) => LaunchMode::Run,
            Some(Commands::Http(options)) => LaunchMode::Quick(QuickCommand {
                mode: match options.mode {
                    QuickHttpMode::Http => IngressMode::Http1,
                    QuickHttpMode::Http2 => IngressMode::Http2,
                },
                port: options.port,
                name: options.name.clone(),
                once: options.once,
            }),
            Some(Commands::Tcp(options)) => LaunchMode::Quick(QuickCommand {
                mode: IngressMode::Tcp,
                port: options.port,
                name: options.name.clone(),
                once: options.once,
            }),
            None => {
                if let Some(port) = self.port {
                    LaunchMode::Quick(QuickCommand {
                        mode: match self.mode {
                            QuickHttpMode::Http => IngressMode::Http1,
                            QuickHttpMode::Http2 => IngressMode::Http2,
                        },
                        port,
                        name: self.name.clone(),
                        once: self.once,
                    })
                } else {
                    LaunchMode::Run
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeMode {
    Tui,
    Headless,
}

#[derive(Debug, Clone)]
pub(crate) enum LaunchMode {
    Run,
    Quick(QuickCommand),
}

#[derive(Debug, Clone)]
pub(crate) struct QuickCommand {
    pub(crate) mode: IngressMode,
    pub(crate) port: u16,
    pub(crate) name: Option<String>,
    pub(crate) once: bool,
}

pub(crate) fn select_runtime_mode(options: &RunOptions) -> RuntimeMode {
    if options.headless {
        RuntimeMode::Headless
    } else if options.tui {
        RuntimeMode::Tui
    } else if !std::io::stdout().is_terminal() {
        RuntimeMode::Headless
    } else {
        RuntimeMode::Tui
    }
}
