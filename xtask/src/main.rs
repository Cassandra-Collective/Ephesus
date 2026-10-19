//! The `cargo xtask` binary: the command line over [`xtask::dispatch`].

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    xtask::dispatch(&args)
}
