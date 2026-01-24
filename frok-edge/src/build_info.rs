use std::sync::OnceLock;

use frok_common::build_info as common_build_info;
use frok_protocol::BuildInfo;

static BUILD_INFO: OnceLock<BuildInfo> = OnceLock::new();

pub fn local_build_info() -> BuildInfo {
    BUILD_INFO
        .get_or_init(|| {
            common_build_info::build_info_from_parts(
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
                option_env!("FROK_GIT_SHA_SHORT"),
            )
        })
        .clone()
}

pub use frok_common::build_info::format_build;
