use std::path::{Path, PathBuf};

fn data_dir() -> Option<PathBuf> {
    if let Ok(root) = std::env::var("FROK_HOME") {
        return Some(PathBuf::from(root));
    }

    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .map(|dir| dir.join("frok"))
}

pub fn data_path(file_name: impl AsRef<Path>) -> PathBuf {
    if let Some(dir) = data_dir() {
        return dir.join(file_name);
    }

    PathBuf::from(file_name.as_ref())
}
