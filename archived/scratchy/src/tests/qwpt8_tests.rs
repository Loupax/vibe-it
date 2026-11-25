// QWPT8: the runner's `--server wptserve` plumbing (area split, ready-line parsing).
use crate::wpt_th_run::*;
use std::fs;
use std::time::Duration;

#[test]
fn reads_the_wptserve_state_written_by_the_script() {
    let d = std::env::temp_dir().join(format!("qwpt8s-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    let st = d.join(".wptserve");
    fs::create_dir_all(&st).unwrap();
    assert!(wptserve_state(&d).unwrap_err().contains("tools/wptserve.sh start"), "no state yet");
    fs::write(st.join("port"), "26960\n").unwrap();
    fs::write(st.join("wire.sock.path"), format!("{}\n", d.join("w.sock").display())).unwrap();
    assert!(wptserve_state(&d).unwrap_err().contains("does not answer"), "socket missing");
    let _l = std::os::unix::net::UnixListener::bind(d.join("w.sock")).unwrap();
    let (port, sock) = wptserve_state(&d).unwrap();
    assert_eq!((port, sock), (26960, d.join("w.sock")));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn the_fetch_and_xhr_areas_belong_to_wptserve_only() {
    for a in ["fetch/api", "fetch/other", "xhr"] {
        assert!(is_wptserve_area(a), "{a}");
    }
    for a in ["dom/nodes", "url", "html/canvas/element", "fetch", "xhr/x"] {
        assert!(!is_wptserve_area(a), "{a}");
    }
}

/// Each server runs only its own areas; naming the other server's area explicitly is an error (exit 2).
#[test]
fn each_server_runs_only_its_own_areas() {
    let d = std::env::temp_dir().join(format!("qwpt8-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("subset.txt"), "# area: url\nurl/a.html\n# area: xhr\nxhr/b.html\n").unwrap();
    let script = d.join("child.sh");
    fs::write(&script, "echo '{\"harness\":\"OK\",\"message\":null,\"tests\":[[\"r\",\"PASS\",null]]}'\n").unwrap();
    let cmd = ChildCmd { program: "sh".into(), args: vec![script.display().to_string()], verbose: false, origin: "http://web-platform.test:1".into() };
    let cfg = |wptserve: bool, area: Option<&str>, update: bool| Config {
        dir: d.clone(),
        update,
        area: area.map(String::from),
        except: None,
        jobs: 2,
        timeout: Duration::from_secs(5),
        verbose: false,
        wptserve,
    };
    assert_eq!(execute(&cfg(false, None, true), &cmd), 0);
    assert!(d.join("expected/url.txt").is_file() && !d.join("expected/xhr.txt").exists(), "mount run skips xhr");
    assert_eq!(execute(&cfg(true, None, true), &cmd), 0);
    assert!(d.join("expected/xhr.txt").is_file());
    assert_eq!(execute(&cfg(false, Some("xhr"), false), &cmd), 2, "xhr needs --server wptserve");
    assert_eq!(execute(&cfg(true, Some("url"), false), &cmd), 2, "url runs on the mount");
    assert_eq!(execute(&cfg(true, Some("xhr"), false), &cmd), 0);
    let _ = fs::remove_dir_all(&d);
}
