use std::env;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=FROK_GIT_SHA");
    println!("cargo:rerun-if-env-changed=FROK_GIT_SHA_SHORT");

    if let Ok(value) = env::var("FROK_GIT_SHA") {
        println!("cargo:rustc-env=FROK_GIT_SHA={value}");
    }
    if let Ok(value) = env::var("FROK_GIT_SHA_SHORT") {
        println!("cargo:rustc-env=FROK_GIT_SHA_SHORT={value}");
    }

    if env::var("FROK_GIT_SHA_SHORT").is_err() {
        if let Some(short) = git_rev_parse(&["rev-parse", "--short=8", "HEAD"]) {
            println!("cargo:rustc-env=FROK_GIT_SHA_SHORT={short}");
        }
    }

    if env::var("FROK_GIT_SHA").is_err() {
        if let Some(full) = git_rev_parse(&["rev-parse", "HEAD"]) {
            println!("cargo:rustc-env=FROK_GIT_SHA={full}");
        }
    }
}

fn git_rev_parse(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() { None } else { Some(value) }
}
