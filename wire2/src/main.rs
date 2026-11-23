//! `wire2`: the proxy binary. Everything lives in the library (src/lib.rs); see NOTES.md.

fn main() -> std::process::ExitCode {
    wire2::proxy_main()
}
