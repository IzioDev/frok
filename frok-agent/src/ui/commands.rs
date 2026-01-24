use frok_protocol::IngressMode;

#[derive(Debug)]
pub enum Command {
    Register {
        name: String,
        local_addr: String,
        mode: IngressMode,
    },
    Unregister {
        name: String,
    },
    ShowLogs,
    HideLogs,
    ToggleLogs,
    ClearLogs,
    Connect,
    OpenGithub,
    Help,
    Quit,
}

#[derive(Debug)]
pub(crate) struct CommandSpec {
    pub(crate) usage: &'static str,
    pub(crate) desc: &'static str,
}

pub(crate) const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        usage: "register <name> <addr> [http|http2|grpc|tcp]",
        desc: "Expose a local service",
    },
    CommandSpec {
        usage: "unregister <name>",
        desc: "Remove a route",
    },
    CommandSpec {
        usage: "logs [on|off|view|toggle]",
        desc: "Open or toggle log view",
    },
    CommandSpec {
        usage: "connect",
        desc: "Connect to the edge",
    },
    CommandSpec {
        usage: "github",
        desc: "Open the GitHub repo",
    },
    CommandSpec {
        usage: "clear",
        desc: "Clear log buffer",
    },
    CommandSpec {
        usage: "help",
        desc: "Show command help",
    },
    CommandSpec {
        usage: "quit",
        desc: "Exit the agent",
    },
];

pub(crate) fn command_specs() -> &'static [CommandSpec] {
    COMMANDS
}

pub(crate) fn parse_command(input: &str) -> Option<Command> {
    let mut parts = input.split_whitespace();
    let cmd = parts
        .next()?
        .trim_start_matches('/')
        .trim_start_matches(':')
        .to_ascii_lowercase();
    match cmd.as_str() {
        "register" => {
            let name = parts.next()?.to_string();
            let local_addr = parts.next()?.to_string();
            let mode = parse_mode(parts.next());
            Some(Command::Register {
                name,
                local_addr,
                mode,
            })
        }
        "unregister" => {
            let name = parts.next()?.to_string();
            Some(Command::Unregister { name })
        }
        "logs" => match parts.next().map(|p| p.to_ascii_lowercase()) {
            None => Some(Command::ToggleLogs),
            Some(option) if option == "on" || option == "show" || option == "view" => {
                Some(Command::ShowLogs)
            }
            Some(option) if option == "off" || option == "hide" => Some(Command::HideLogs),
            Some(option) if option == "toggle" => Some(Command::ToggleLogs),
            _ => Some(Command::ToggleLogs),
        },
        "connect" | "reconnect" => Some(Command::Connect),
        "github" | "gh" => Some(Command::OpenGithub),
        "clear" | "cls" => Some(Command::ClearLogs),
        "help" | "?" => Some(Command::Help),
        "quit" | "exit" | "q" => Some(Command::Quit),
        "back" => Some(Command::HideLogs),
        _ => None,
    }
}

fn parse_mode(token: Option<&str>) -> IngressMode {
    match token.map(|t| t.to_ascii_lowercase()) {
        Some(mode) if mode == "http2" || mode == "h2" || mode == "grpc" => IngressMode::Http2,
        Some(mode) if mode == "tcp" => IngressMode::Tcp,
        _ => IngressMode::Http1,
    }
}
