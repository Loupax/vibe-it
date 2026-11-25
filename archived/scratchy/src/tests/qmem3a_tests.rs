// QMEM3a: glibc arena cap. glibc's own behaviour (fewer arenas) cannot be
// unit-tested; this checks the hook is callable and parses the env override.
#[allow(unused_imports)]
use super::*;
use crate::malloc_arenas::{arenas_from, install, DEFAULT_ARENAS};

#[test]
fn qmem3a_env_override_parsing() {
    assert_eq!(arenas_from(None), Some(DEFAULT_ARENAS));
    assert_eq!(arenas_from(Some("0")), None);
    assert_eq!(arenas_from(Some("4")), Some(4));
    assert_eq!(arenas_from(Some("junk")), Some(DEFAULT_ARENAS));
}

#[test]
fn qmem3a_install_is_callable() {
    // Do not mutate the env (shared test process); just call the hook.
    let r = install();
    if cfg!(all(target_os = "linux", target_env = "gnu")) {
        let want = arenas_from(std::env::var("SCRATCHY_MALLOC_ARENAS").ok().as_deref());
        assert_eq!(r, want);
    } else {
        assert_eq!(r, None);
    }
}
