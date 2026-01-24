use frok_protocol::BuildInfo;

pub fn build_info_from_parts(
    name: &'static str,
    version: &'static str,
    commit: Option<&'static str>,
) -> BuildInfo {
    BuildInfo {
        name: name.to_string(),
        version: version.to_string(),
        commit: commit.map(|value| value.to_string()),
    }
}

pub fn format_build(info: &BuildInfo) -> String {
    match info.commit.as_deref() {
        Some(commit) => format!("v{} ({commit})", info.version),
        None => format!("v{}", info.version),
    }
}
