mod opt;
mod subcommand;

pub use opt::Opt;
use std::path::PathBuf;

lazy_static::lazy_static! {
    static ref PROJECT_DIRS: directories::ProjectDirs =
        directories::ProjectDirs::from(
            "org",
            "senkbeil",
            "distant",
        ).expect("Failed to find valid home directory path");
    static ref SESSION_PATH: PathBuf = PROJECT_DIRS.cache_dir().join("session");
}

/// Main entrypoint into the program
pub fn run() {
    let opt = Opt::load();
    init_logging(&opt.common);
    if let Err(x) = opt.subcommand.run() {
        eprintln!("{}", x);
    }
}

pub fn init_logging(opt: &opt::CommonOpt) {
    stderrlog::new()
        .module("distant")
        .quiet(opt.quiet)
        .verbosity(opt.verbose as usize)
        .timestamp(stderrlog::Timestamp::Off)
        .init()
        .unwrap();
}
