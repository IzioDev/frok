use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use url::Url;

pub(crate) struct ProcessHandle {
    child: Child,
    drain_tasks: Vec<JoinHandle<()>>,
    stdout_log: Option<PathBuf>,
    stderr_log: Option<PathBuf>,
}

impl ProcessHandle {
    pub(crate) async fn kill(&mut self) -> Result<()> {
        if self
            .child
            .try_wait()
            .context("check child status")?
            .is_none()
        {
            let _ = self.child.kill().await;
        }
        let _ = self.child.wait().await;
        for task in &self.drain_tasks {
            task.abort();
        }
        Ok(())
    }

    pub(crate) fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>> {
        self.child.try_wait().context("check child status")
    }

    pub(crate) fn log_paths(&self) -> (Option<PathBuf>, Option<PathBuf>) {
        (self.stdout_log.clone(), self.stderr_log.clone())
    }
}

pub(crate) fn spawn_process(
    mut cmd: Command,
    name: &str,
    log_dir: Option<&Path>,
) -> Result<ProcessHandle> {
    if should_inherit_stdio(name) {
        cmd.stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        let child = cmd.spawn().with_context(|| format!("spawn {name}"))?;
        return Ok(ProcessHandle {
            child,
            drain_tasks: Vec::new(),
            stdout_log: None,
            stderr_log: None,
        });
    }

    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().with_context(|| format!("spawn {name}"))?;

    let stdout = child.stdout.take().context("capture stdout")?;
    let stderr = child.stderr.take().context("capture stderr")?;

    let stdout_log = log_dir.map(|dir| dir.join(format!("{name}.stdout.log")));
    let stderr_log = log_dir.map(|dir| dir.join(format!("{name}.stderr.log")));

    let drain_tasks = vec![
        spawn_drain(stdout, None, stdout_log.clone(), should_echo(name)),
        spawn_drain(stderr, None, stderr_log.clone(), should_echo(name)),
    ];

    Ok(ProcessHandle {
        child,
        drain_tasks,
        stdout_log,
        stderr_log,
    })
}

pub(crate) fn spawn_agent(
    mut cmd: Command,
    name: &str,
    log_dir: Option<&Path>,
) -> Result<(ProcessHandle, oneshot::Receiver<String>)> {
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().with_context(|| format!("spawn {name}"))?;

    let stdout = child.stdout.take().context("capture stdout")?;
    let stderr = child.stderr.take().context("capture stderr")?;

    let (url_tx, url_rx) = oneshot::channel();
    let stdout_log = log_dir.map(|dir| dir.join(format!("{name}.stdout.log")));
    let stderr_log = log_dir.map(|dir| dir.join(format!("{name}.stderr.log")));
    let drain_tasks = vec![
        spawn_drain(stdout, Some(url_tx), stdout_log.clone(), should_echo(name)),
        spawn_drain(stderr, None, stderr_log.clone(), should_echo(name)),
    ];

    Ok((
        ProcessHandle {
            child,
            drain_tasks,
            stdout_log,
            stderr_log,
        },
        url_rx,
    ))
}

fn spawn_drain(
    io: impl tokio::io::AsyncRead + Unpin + Send + 'static,
    url_tx: Option<oneshot::Sender<String>>,
    log_path: Option<PathBuf>,
    echo: bool,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut reader = BufReader::new(io).lines();
        let mut url_tx = url_tx;
        let mut log_file = match log_path {
            Some(path) => tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .await
                .ok(),
            None => None,
        };
        while let Ok(Some(line)) = reader.next_line().await {
            if let Some(file) = log_file.as_mut() {
                let _ = file.write_all(line.as_bytes()).await;
                let _ = file.write_all(b"\n").await;
            }
            if echo {
                println!("{line}");
            }
            if let Some(tx) = url_tx.take() {
                if let Some(host) = extract_url_host(&line) {
                    let _ = tx.send(host);
                } else {
                    url_tx = Some(tx);
                }
            }
        }
    })
}

fn should_echo(name: &str) -> bool {
    let Ok(value) = std::env::var("FROK_BENCH_ECHO") else {
        return false;
    };
    if value == "1" || value.eq_ignore_ascii_case("true") {
        return true;
    }
    value.eq_ignore_ascii_case(name)
}

fn should_inherit_stdio(name: &str) -> bool {
    let Ok(value) = std::env::var("FROK_BENCH_STDIO") else {
        return false;
    };
    if value == "1" || value.eq_ignore_ascii_case("true") {
        return true;
    }
    value.eq_ignore_ascii_case(name)
}

fn extract_url_host(line: &str) -> Option<String> {
    let cleaned = strip_ansi(line).trim().to_string();
    for scheme in ["http://", "https://", "tcp://"] {
        if let Some(idx) = cleaned.find(scheme) {
            let tail = &cleaned[idx..];
            let token = tail.split_whitespace().next().unwrap_or(tail);
            let token = token.trim_matches(|c: char| c.is_ascii_control());
            if let Ok(url) = Url::parse(token) {
                if let Some(host) = url.host_str() {
                    return Some(host.to_string());
                }
            }
        }
    }
    None
}

fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if matches!(chars.peek(), Some('[')) {
                let _ = chars.next();
                while let Some(c) = chars.next() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
                continue;
            }
        }
        if !ch.is_control() {
            out.push(ch);
        }
    }
    out
}
