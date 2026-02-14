# Frok

> [!CAUTION]
> **Alpha software.** Expect breaking changes, incomplete features, instability and bugs.

Frok is a self-hosted edge/agent tunneling system that exposes local HTTP (1.1/2) and TCP services
through a public edge using a QUIC control/data plane. It includes an interactive TUI agent and a
server-side edge that handles ingress, routing, and authentication.

## Overview

- **Agent (`frok`)** runs near your local service, registers routes, and forwards requests.
- **Edge (`frok-edge`)** accepts public traffic and forwards it to the correct agent.
- **Protocol (`frok-protocol`)** defines a framed, Borsh-serialized wire protocol over QUIC.

## Architecture

```
External Client -> [Edge HTTP/TCP ingress] -> QUIC -> [Agent] -> Local service
```

- The edge listens on:
  - a **QUIC** socket for agent connections (UDP)
  - an **HTTP ingress** socket for public HTTP/1.1 and HTTP/2 traffic (TCP)
- Routes are host-based (`name.<public-domain>`). The agent builds route names from the
  `--public-domain` value (or falls back to the `--edge` host) and the route label you provide.
- TCP routes are mapped to ephemeral public ports. The agent prints `tcp://host:port` when ready.

## Key Features

- QUIC transport for agent connections
- HTTP/1.1, HTTP/2 (including gRPC), and raw TCP routing
- Host-based route registration and conflict detection
- TUI and headless modes for the agent
- OIDC or key-based authentication on the edge
- Route persistence on the agent and identity/route storage on the edge

## Getting Started

### Install the agent (binary)

Linux/macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/IzioDev/frok/master/install.sh | sh
```

Windows (PowerShell):

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -Command "irm https://raw.githubusercontent.com/IzioDev/frok/master/install.ps1 | iex"
```

Overrides:

- `FROK_VERSION=1.2.3` (downloads from the matching release tag `v1.2.3`)
- `FROK_INSTALL_DIR=/custom/bin` (or `%FROK_INSTALL_DIR%` on Windows)

### Prerequisites

- Recent Rust toolchain with 2024 edition support
- UDP connectivity from agent to edge (QUIC)
- A public domain for the edge (for host-based routing)

### Build

```bash
cargo build
```

Commands below use the binary names (`frok` and `frok-edge`). If you built from source and they are
not on your `PATH`, run them from `target/debug`:

```bash
./target/debug/frok --help
./target/debug/frok-edge --help
```

### Run the edge (development)

```bash
frok-edge --insecure \
  --quic-addr 0.0.0.0:5000 \
  --http-addr 0.0.0.0:8080
```

Notes:

- In debug builds, the edge can generate a dev TLS cert automatically if you do not provide one.
- In release builds, `--tls-cert` and `--tls-key` (or the env vars) are required.

### Run the agent (TUI)

```bash
# By default it uses the public edge.frok.it agent
# Specify --edge example.com option to connect to your own
frok
```

### Quick expose (no TUI)

```bash
# Expose local HTTP service on port 3000
frok 3000

# Expose local HTTP/2 (gRPC) service
frok http 3000 --mode http2

# Expose local TCP service
frok tcp 22 --name ssh
```

Use `--once` to register, print the URL, and exit.

### Agent TUI commands

- `register <name> <addr> [http|http2|grpc|tcp]`
- `unregister <name>`
- `logs [on|off|view|toggle]`
- `connect`
- `clear`
- `quit`

## Configuration

Both binaries read `.env` automatically on startup (via `dotenvy`).

### Edge (`frok-edge`)

| Flag                  | Env                           | Default          | Notes                             |
| --------------------- | ----------------------------- | ---------------- | --------------------------------- |
| `--quic-addr`         | `FROK_EDGE_QUIC_ADDR`         | `127.0.0.1:5000` | QUIC (UDP) listen address         |
| `--http-addr`         | `FROK_EDGE_HTTP_ADDR`         | `127.0.0.1:8080` | HTTP ingress address              |
| `--tls-cert`          | `FROK_EDGE_TLS_CERT`          | (none)           | TLS cert chain (PEM/DER)          |
| `--tls-key`           | `FROK_EDGE_TLS_KEY`           | (none)           | TLS key (PEM/DER, PKCS#8 for DER) |
| `--insecure`          | `FROK_EDGE_INSECURE`          | `false`          | Disable auth (dev only)           |
| `--oidc-required`     | `FROK_EDGE_OIDC_REQUIRED`     | `false`          | Require OIDC auth                 |
| `--key-auth-required` | `FROK_EDGE_KEY_AUTH_REQUIRED` | `false`          | Require key auth                  |
| `--oidc-issuer`       | `FROK_EDGE_OIDC_ISSUER`       | (none)           | OIDC issuer URL                   |
| `--oidc-audience`     | `FROK_EDGE_OIDC_AUDIENCE`     | (none)           | OIDC audience/client ID           |
| `--oidc-allowed-algs` | `FROK_EDGE_OIDC_ALLOWED_ALGS` | `RS256,ES256`    | Allowed JWT algs                  |
| `--oidc-jwks-ttl`     | `FROK_EDGE_OIDC_JWKS_TTL`     | `300`            | JWKS cache TTL (seconds)          |
| `--key-tofu`          | `FROK_EDGE_KEY_TOFU`          | `true`           | Trust on first use for key auth   |

Rules:

- Exactly one of `--oidc-required` or `--key-auth-required` must be set unless `--insecure` is used.

### Agent (`frok`)

| Flag              | Env                  | Default                                     | Notes                             |
| ----------------- | -------------------- | ------------------------------------------- | --------------------------------- |
| `--edge`          | `FROK_EDGE`          | `edge.frok.it:443`                          | Edge host:port for QUIC           |
| `--public-domain` | `FROK_PUBLIC_DOMAIN` | (edge host; if edge.<domain> then <domain>) | Public domain for route hostnames |
| `--agent-label`   | `FROK_AGENT_LABEL`   | `agent-1`                                   | Label used during auth/hello      |
| `--headless`      | `FROK_HEADLESS`      | `false`                                     | Disable TUI                       |
| `--tui`           | `FROK_TUI`           | `false`                                     | Force TUI even without TTY        |
| `--log-file`      | `FROK_LOG_FILE`      | `false`                                     | Enable file logs in headless mode |

Proxy tuning (agent only):

- `FROK_HTTP1_POOL_SIZE` (default: 4)
- `FROK_PROXY_QUEUE_DEPTH` (default: 64)

## Authentication

The edge enforces one of the following modes (unless `--insecure` is used):

- **OIDC**: the agent opens a browser for login, performs PKCE, and caches the ID token and refresh
  token locally. The edge validates the token against the issuer's JWKS.
- **Key auth**: the agent generates an Ed25519 key pair and signs a nonce during login. The edge
  tracks key fingerprints and can enforce trust-on-first-use (TOFU) or pre-trusted keys.

## Data and Storage

Data location is controlled by `FROK_HOME`. If unset, the OS data directory is used (and a `frok`
subfolder is created).

- **Edge**: `edge.db` (Redb) stores identities, route ownership, and key trust records.
- **Agent**:
  - `default.toml`: persisted routes
  - `agent-key.bin`: Ed25519 key for key auth
  - `oidc-cache.bin`: cached OIDC token
  - `config.toml`: UI preferences (theme/texture)

Dev TLS assets (for debug builds) are stored in `target/quinn-dev-certs`.

## Logging

- Set `FROK_LOG` or `RUST_LOG` for log filtering (e.g., `info`, `debug`).
- Logs can be written to `./logs`:
  - `frok-edge` always writes `edge.log.*`
  - `frok` writes `agent.log.*` when TUI is active or when `--log-file` is set in headless mode

## Benchmarks

Use the internal bench to simulate one edge + N agents + proxied services:

```bash
cargo bench -p frok-bench
```

Enable the heavy scale bench:

```bash
cargo bench -p frok-bench --features bench-heavy --bench ceremony_heavy
```

## Protocol Notes

- Messages are Borsh-serialized and framed with a 4-byte little-endian length prefix.
- Max frame size: 10 MB
- Max HTTP body chunk: 256 KB

## Deployment Notes

- HTTP ingress is plain TCP (no TLS). If you want HTTPS, place a TLS terminator in front of the
  edge's HTTP ingress port.
- QUIC uses UDP; ensure UDP is allowed between agent and edge.

## Troubleshooting

- **Edge fails to start in release**: provide `--tls-cert` and `--tls-key` (or env vars).
- **"unknown host" on HTTP**: the route is not registered or the Host header does not match.
- **"route already taken"**: the hostname is owned by another authenticated subject.
- **Auth errors**: verify OIDC issuer/audience, or clear `agent-key.bin`/`oidc-cache.bin` if needed.

## Workspace Layout

- `frok-agent/` - CLI/TUI agent (binary: `frok`)
- `frok-edge/` - edge server (binary: `frok-edge`)
- `frok-protocol/` - shared wire protocol types
- `frok-common/` - common utilities (logging, paths, HTTP helpers)
- `dev-tls/` - dev TLS generation for debug builds

## Legal

- Code license: `LICENSE`
- Hosted service terms: `TERMS.md`
- Hosted service privacy policy: `PRIVACY.md`
- Hosted service acceptable use policy: `ACCEPTABLE_USE.md`
