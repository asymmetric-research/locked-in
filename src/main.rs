use std::process;

fn main() {
    process::exit(locked_in::cli::run(std::env::args()));
}
