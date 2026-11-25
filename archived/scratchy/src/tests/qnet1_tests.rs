// QNET1: scratchy cannot speak to the internet.
#[allow(unused_imports)]
use super::*;

/// Child half of `socket_filter_allows_only_unix_sockets`: runs only when the
/// parent re-executes this test binary with the env var set (the filter is
/// irreversible, so it must never be installed in the shared test process).
#[test]
fn qnet1_child_probe() {
    if std::env::var_os("SCRATCHY_QNET1_CHILD").is_none() {
        return;
    }
    let st = crate::netlock::install();
    if st != crate::netlock::Status::Installed {
        println!("QNET1-SKIP {st:?}");
        return;
    }
    let errno = || std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    for (name, dom, ty) in [
        ("tcp4", libc::AF_INET, libc::SOCK_STREAM),
        ("tcp6", libc::AF_INET6, libc::SOCK_STREAM),
        ("udp4", libc::AF_INET, libc::SOCK_DGRAM),
        ("udp6", libc::AF_INET6, libc::SOCK_DGRAM),
        ("packet", libc::AF_PACKET, libc::SOCK_RAW),
        ("netlink", libc::AF_NETLINK, libc::SOCK_RAW),
        ("raw", libc::AF_INET, libc::SOCK_RAW),
    ] {
        let fd = unsafe { libc::socket(dom, ty, 0) };
        assert_eq!(fd, -1, "{name} socket() must fail");
        assert_eq!(errno(), libc::EAFNOSUPPORT, "{name}");
    }
    // socketpair of a non-unix family fails too
    let mut sv = [0i32; 2];
    assert_eq!(unsafe { libc::socketpair(libc::AF_INET, libc::SOCK_STREAM, 0, sv.as_mut_ptr()) }, -1);
    assert_eq!(errno(), libc::EAFNOSUPPORT);
    // threads inherit it
    let t = std::thread::spawn(move || {
        let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
        (fd, std::io::Error::last_os_error().raw_os_error())
    });
    assert_eq!(t.join().unwrap(), (-1, Some(libc::EAFNOSUPPORT)));
    // std's own TCP/UDP fail, unix sockets (path and pair) and flags still work
    assert!(std::net::TcpStream::connect("127.0.0.1:9").is_err());
    assert!(std::net::UdpSocket::bind("127.0.0.1:0").is_err());
    let (a, b) = std::os::unix::net::UnixStream::pair().expect("unix socketpair must work");
    let path = std::env::temp_dir().join(format!("qnet1-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let l = std::os::unix::net::UnixListener::bind(&path).expect("unix bind must work");
    let c = std::os::unix::net::UnixStream::connect(&path).expect("unix connect must work");
    let _ = std::fs::remove_file(&path);
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK, 0) };
    assert!(fd >= 0, "AF_UNIX with flags must work");
    unsafe { libc::close(fd) };
    drop((a, b, l, c));
    println!("QNET1-OK");
}

#[test]
fn socket_filter_allows_only_unix_sockets() {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tests::qnet1_tests::qnet1_child_probe", "--nocapture", "--test-threads=1"])
        .env("SCRATCHY_QNET1_CHILD", "1")
        .output()
        .unwrap();
    let so = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "child failed:\n{so}\n{}", String::from_utf8_lossy(&out.stderr));
    if so.contains("QNET1-SKIP") {
        eprintln!("SKIP: seccomp unavailable here: {so}");
        return;
    }
    assert!(so.contains("QNET1-OK"), "{so}");
}

/// No network crate in the normal (non-dev, host) dependency tree.
#[test]
fn no_network_crate_in_normal_dependencies() {
    let run = |offline: bool| {
        let mut c = std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        c.args(["tree", "-e", "normal", "--prefix", "none", "--format", "{p}"]);
        if offline {
            c.arg("--offline");
        }
        c.current_dir(env!("CARGO_MANIFEST_DIR")).output().ok().filter(|o| o.status.success())
    };
    let Some(out) = run(true).or_else(|| run(false)) else {
        eprintln!("SKIP: cargo tree unavailable");
        return;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let names: Vec<&str> = text.lines().filter_map(|l| l.split_whitespace().next()).collect();
    assert!(names.len() > 20 && names.contains(&"scratchy"), "cargo tree output looks wrong: {text}");
    const BANNED: &[&str] = &[
        "ureq", "reqwest", "hyper", "hyper-util", "hyper-tls", "h2", "tokio", "tokio-native-tls", "rustls", "rustls-native-certs",
        "native-tls", "openssl", "openssl-sys", "curl", "curl-sys", "isahc", "surf", "attohttpc", "minreq", "tiny_http",
        "async-std", "smol", "socket2", "mio", "tungstenite", "tokio-tungstenite", "hickory-resolver", "trust-dns-resolver",
        "c-ares", "webpki-roots", "rustls-webpki", "quinn", "libp2p", "zmq", "nng",
    ];
    let bad: Vec<&&str> = names.iter().filter(|n| BANNED.contains(n)).collect();
    assert!(bad.is_empty(), "network crate(s) in the normal dependency tree (QNET1 / AGENTS.md rule): {bad:?}. Ask the user first.");
}

/// No std::net sockets / DNS in the sources (tests and tools excluded).
#[test]
fn sources_do_not_use_inet_sockets() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = 0;
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        if p.extension().is_none_or(|x| x != "rs" && x != "js") || name == "tests.rs" {
            continue;
        }
        files += 1;
        let text = std::fs::read_to_string(&p).unwrap();
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            for bad in ["TcpStream", "TcpListener", "UdpSocket", "to_socket_addrs", "lookup_host", "getaddrinfo", "gethostbyname", "connect_timeout("] {
                assert!(!code.contains(bad), "{name}:{} uses `{bad}`: scratchy talks only to wire's unix socket (QNET1)", n + 1);
            }
            // every `::connect(` must be a UnixStream one
            if code.contains("::connect(") {
                assert!(code.contains("UnixStream::connect("), "{name}:{} non-unix connect(): {line}", n + 1);
            }
        }
    }
    assert!(files > 20, "scanned only {files} files");
}

/// QNET1 (2): ffmpeg must not open nested URLs; the option is set for mpv.
#[test]
fn mpv_options_whitelist_ffmpeg_protocols() {
    let opts = crate::media_net::MPV_OPTIONS;
    for k in ["demuxer-lavf-o", "stream-lavf-o"] {
        let v = opts.iter().find(|(n, _)| *n == k).unwrap_or_else(|| panic!("{k} missing")).1;
        assert_eq!(v, "protocol_whitelist=%11%crypto,data");
    }
}

/// The help page tells how to verify the lock from outside.
#[test]
fn help_page_documents_the_network_lock() {
    let help = include_str!("../help.html");
    assert!(help.contains("unshare -rn") && help.contains("unix sockets only"));
}

/// An HLS playlist served through the fake wire that names an http:// segment on
/// a local TCP listener: mpv/ffmpeg must not connect to it. (This process has no
/// seccomp filter, so this proves the protocol whitelist on its own.)
#[cfg(feature = "media")]
#[test]
fn hls_playlist_with_http_segment_opens_no_socket() {
    use crate::media_el::*;
    use crate::media_net::MediaMode;
    if !crate::media_mpv::libmpv_available() {
        eprintln!("SKIP: libmpv.so.2 not available");
        return;
    }
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.set_nonblocking(true).unwrap();
    let port = l.local_addr().unwrap().port();
    let playlist = format!("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:2.0,\nhttp://127.0.0.1:{port}/seg0.ts\n#EXT-X-ENDLIST\n");
    let w = qm1e_wire(Qm1eCfg { file: playlist.into_bytes(), ..Default::default() });
    let backend = crate::media_mpv::MpvBackend::new(BackendCtx { socket: Some(w.path.clone()), policy: Default::default() });
    let mut c = MediaCore::new(Box::new(backend));
    c.muted = true;
    c.load(vec![Candidate { url: "https://cdn.test/x.m3u8".into(), mime: String::new() }], true, MediaMode::NoCors, "https://cdn.test/".into());
    let mut now = 0.0;
    for _ in 0..150 {
        now += 20.0;
        c.pump(now, std::time::Duration::from_millis(20));
        let _ = c.take_outs();
        assert!(matches!(l.accept(), Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock), "ffmpeg connected to the http:// segment");
        if c.error.is_some() {
            break;
        }
    }
    assert!(matches!(l.accept(), Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock));
}
