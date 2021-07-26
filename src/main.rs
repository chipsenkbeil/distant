use distant::Opt;

#[tokio::main]
async fn main() {
    let opt = Opt::load();
    distant::init_logging(&opt.common);
    if let Err(x) = opt.subcommand.run().await {
        eprintln!("{}", x);
    }
}
