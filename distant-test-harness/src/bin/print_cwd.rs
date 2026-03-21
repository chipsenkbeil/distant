//! Prints the current working directory to stdout and exits.
//! Cross-platform replacement for `pwd` / `cd` in current_dir tests.

fn main() {
    println!("{}", std::env::current_dir().unwrap().display());
}
