fn main() {
    // QNET1: first thing, before any thread or library: unix sockets only.
    scratchy::netlock::install_and_log();
    scratchy::run();
}
