#[allow(dead_code)]
mod api;
#[allow(dead_code)]
mod config;
#[allow(dead_code)]
mod error;

fn main() {
    println!("flarion v{}", env!("CARGO_PKG_VERSION"));
}
