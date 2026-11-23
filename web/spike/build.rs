// cef-dll-sys copies libcef.so and its resources to "OUT_DIR/../../..", meant to
// be target/<profile>/ next to the binary. With cargo's current build-dir layout
// (target/<profile>/build/<crate>/<hash>/out) that is target/<profile>/build/,
// so the binary finds nothing. Hard-link the files from the CEF dir (cached under
// $CEF_PATH) next to the binary instead: cheap on the same filesystem, else a copy.
// cef-dll-sys sets no rpath either: $ORIGIN finds libcef.so.
use std::{fs, io, path::Path};

fn link_files(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for e in fs::read_dir(src)? {
        let e = e?;
        if !e.file_type()?.is_file() {
            continue;
        }
        let to = dst.join(e.file_name());
        let _ = fs::remove_file(&to);
        if fs::hard_link(e.path(), &to).is_err() {
            fs::copy(e.path(), &to)?;
        }
    }
    Ok(())
}

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rustc-link-arg=-Wl,-rpath,$ORIGIN");

    // Set by cef-dll-sys (links = "cef_dll_wrapper"), whose build script ran first.
    let cef_dir = std::env::var("DEP_CEF_DLL_WRAPPER_CEF_DIR").expect("cef-dll-sys gave no CEF_DIR");
    let cef_dir = Path::new(&cef_dir);
    println!("cargo::rerun-if-changed={}", cef_dir.display());
    // libcef.so must come before libc.so.6 in the binary's DT_NEEDED list: Chromium
    // wraps close() and finds the real one with dlsym(RTLD_NEXT, "close"), which only
    // searches libraries loaded AFTER libcef. Since W6 pulls in the `libc` crate (via
    // ring), its `-lc` landed before cef-dll-sys's `-lcef` and every start died at once
    // ("close symbol missing", SIGTRAP in ContentMainRunnerImpl::Initialize). This
    // crate's own native libraries are linked before any dependency's, so naming libcef
    // here puts it first.
    println!("cargo::rustc-link-search=native={}", cef_dir.display());
    println!("cargo::rustc-link-lib=dylib=cef");

    // target/<profile>/ is the parent of the "build" dir that holds OUT_DIR.
    let out = std::env::var("OUT_DIR").unwrap();
    let build = Path::new(&out)
        .ancestors()
        .find(|p| p.file_name() == Some("build".as_ref()))
        .expect("OUT_DIR is not under a build/ dir");
    let profile_dir = build.parent().unwrap();
    link_files(cef_dir, profile_dir).expect("linking CEF runtime files");
    link_files(&cef_dir.join("locales"), &profile_dir.join("locales")).expect("linking CEF locales");
}
