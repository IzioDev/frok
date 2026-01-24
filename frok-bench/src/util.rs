use std::io;
use std::net::{SocketAddr, TcpListener, UdpSocket};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .map(PathBuf::from)
        .unwrap_or(manifest_dir)
}

pub fn bin_path(root: &Path, name: &str, profile: &str) -> PathBuf {
    let mut bin_name = name.to_string();
    if cfg!(windows) {
        bin_name.push_str(".exe");
    }
    root.join("target").join(profile).join(bin_name)
}

pub fn ensure_dir(path: &Path) -> io::Result<()> {
    std::fs::create_dir_all(path)
}

pub fn pick_free_tcp_addr() -> io::Result<SocketAddr> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    drop(listener);
    Ok(addr)
}

pub fn pick_free_udp_addr() -> io::Result<SocketAddr> {
    let socket = UdpSocket::bind("127.0.0.1:0")?;
    let addr = socket.local_addr()?;
    drop(socket);
    Ok(addr)
}

pub fn new_run_dir(root: &Path) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    root.join("target")
        .join("frok-bench")
        .join(format!("run-{millis}"))
}
