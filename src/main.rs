use distant::Opt;

fn main() {
    let opt = Opt::load();
    distant::init_logging(&opt.common);
    println!("Hello, world!");
}
