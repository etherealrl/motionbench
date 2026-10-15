// the commit this build came from, so a saved result says which code made it, with -dirty when
// the core's tracked files differed from it. a build with no git beside it says unknown
fn main() {
    for watched in [".git/logs/HEAD", ".git/index", "src"] {
        println!("cargo:rerun-if-changed={watched}");
    }
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .and_then(|out| String::from_utf8(out.stdout).ok())
    };
    let revision = match git(&["rev-parse", "--short=12", "HEAD"]) {
        Some(commit) => {
            let dirty = git(&["status", "--porcelain", "--untracked-files=no", "--", "src"])
                .is_none_or(|changes| !changes.trim().is_empty());
            format!("{}{}", commit.trim(), if dirty { "-dirty" } else { "" })
        }
        None => "unknown".into(),
    };
    println!("cargo:rustc-env=MOTION_BENCH_REVISION={revision}");
}
