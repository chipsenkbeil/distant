mod opt;
mod subcommand;

pub use opt::Opt;

pub fn init_logging(opt: &opt::CommonOpt) {
    stderrlog::new()
        .module("distant")
        .quiet(opt.quiet)
        .verbosity(opt.verbose as usize)
        .timestamp(stderrlog::Timestamp::Off)
        .init()
        .unwrap();
}
