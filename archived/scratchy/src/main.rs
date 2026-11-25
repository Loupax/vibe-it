fn main() {
    // QMEM3a: before any thread exists, so every thread sees the arena cap.
    scratchy::malloc_arenas::install();
    // QNET1: first thing, before any thread or library: unix sockets only.
    scratchy::netlock::install_and_log();
    #[cfg(feature = "memprof")]
    scratchy::memprof::start(); // QMEM3g
    scratchy::run();
}
