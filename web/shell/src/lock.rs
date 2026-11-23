//! C-SINGLE: Chromium's profile lock. A browser process holds `<profile>/SingletonLock`, a
//! symlink whose target is `<hostname>-<pid>` (with `SingletonCookie` and `SingletonSocket`
//! beside it). A second process on the same profile does not start: it hands its command line
//! to the first one and exits, which CEF reports as a failed `initialize`.
//!
//! Before the shell starts the host it looks at that lock (after asking a running web to open
//! the URL, forget.rs): a lock whose process is dead is removed (Chromium would take it over
//! itself; removing it here gives a log line and a clean start), a lock whose process lives is
//! an error naming the profile and the pid. A live lock is never touched.

use std::path::{Path, PathBuf};

pub const LOCK_FILES: &[&str] = &["SingletonLock", "SingletonCookie", "SingletonSocket"];

#[derive(Debug, PartialEq, Eq)]
pub struct Holder {
    pub host: String,
    pub pid: u32,
}

/// `<hostname>-<pid>` (the hostname may contain dashes: the pid is after the last one).
pub fn parse_target(target: &str) -> Option<Holder> {
    let (host, pid) = target.rsplit_once('-')?;
    let pid = pid.parse::<u32>().ok().filter(|p| *p > 0)?;
    (!host.is_empty()).then(|| Holder {
        host: host.to_string(),
        pid,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum Lock {
    /// No lock.
    Free,
    /// The holder's process is gone: the lock is stale.
    Stale { holder: Holder },
    /// The holder is running (`comm` is its name, when readable), or lives on another host.
    Held {
        holder: Holder,
        comm: Option<String>,
        other_host: bool,
    },
    /// A lock whose target is not `<host>-<pid>`.
    Unreadable { target: String },
}

pub fn inspect(profile: &Path, proc_root: &Path, own_host: &str) -> Lock {
    let Ok(target) = std::fs::read_link(profile.join(LOCK_FILES[0])) else {
        return Lock::Free;
    };
    let target = target.to_string_lossy().into_owned();
    let Some(holder) = parse_target(&target) else {
        return Lock::Unreadable { target };
    };
    if holder.host != own_host {
        // Another machine (a shared home directory): not ours to judge.
        return Lock::Held {
            holder,
            comm: None,
            other_host: true,
        };
    }
    let dir = proc_root.join(holder.pid.to_string());
    if !dir.exists() {
        return Lock::Stale { holder };
    }
    let comm = std::fs::read_to_string(dir.join("comm")).ok().map(|c| c.trim_end().to_string());
    Lock::Held {
        holder,
        comm,
        other_host: false,
    }
}

pub fn own_host(proc_root: &Path) -> String {
    std::fs::read_to_string(proc_root.join("sys/kernel/hostname"))
        .map(|h| h.trim_end().to_string())
        .unwrap_or_default()
}

/// Removes the lock files of a stale lock, by explicit name.
pub fn remove_stale(profile: &Path) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    for f in LOCK_FILES {
        let p = profile.join(f);
        if std::fs::remove_file(&p).is_ok() {
            removed.push(p);
        }
    }
    removed
}

/// The message for a lock that stays (what is wrong and what to do).
pub fn refusal(profile: &Path, lock: &Lock) -> Option<String> {
    let dir = profile.display();
    match lock {
        Lock::Held {
            holder,
            comm,
            other_host: false,
        } => Some(format!(
            "the profile {dir} is in use by process {} ({}), which is not a web with a command socket \
             for this state directory.\n  If it is a leftover web-host-cef, stop it (kill {}); otherwise use another --state-dir.",
            holder.pid,
            comm.as_deref().unwrap_or("name unknown"),
            holder.pid
        )),
        Lock::Held {
            holder, other_host: true, ..
        } => Some(format!(
            "the profile {dir} is locked by process {} on host {:?}, not this machine. If that session is over, \
             remove {dir}/SingletonLock yourself; otherwise use another --state-dir.",
            holder.pid, holder.host
        )),
        Lock::Unreadable { target } => Some(format!(
            "the profile {dir} has a lock ({dir}/SingletonLock -> {target:?}) in a form web does not know. \
             If no browser uses it, remove {dir}/SingletonLock; otherwise use another --state-dir."
        )),
        _ => None,
    }
}

/// Checks the profile before the host starts: Ok(Some(line)) when a stale lock was removed (to
/// log), Ok(None) when free, Err(message) when the profile is in use.
pub fn check(profile: &Path, proc_root: &Path) -> Result<Option<String>, String> {
    let lock = inspect(profile, proc_root, &own_host(proc_root));
    if let Some(m) = refusal(profile, &lock) {
        return Err(m);
    }
    match lock {
        Lock::Stale { holder } => {
            let removed = remove_stale(profile);
            Ok(Some(format!(
                "removed the stale profile lock in {} (its process {} is gone; removed {})",
                profile.display(),
                holder.pid,
                removed
                    .iter()
                    .filter_map(|p| p.file_name())
                    .map(|n| n.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(", ")
            )))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Tmp {
            let p = std::env::temp_dir().join(format!("web-lock-test-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(p.join("profile")).unwrap();
            std::fs::create_dir_all(p.join("proc/sys/kernel")).unwrap();
            std::fs::write(p.join("proc/sys/kernel/hostname"), "my-box\n").unwrap();
            Tmp(p)
        }
        fn profile(&self) -> PathBuf {
            self.0.join("profile")
        }
        fn proc(&self) -> PathBuf {
            self.0.join("proc")
        }
        fn lock(&self, target: &str) {
            symlink(target, self.profile().join("SingletonLock")).unwrap();
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn targets_parse_with_dashes_in_the_host() {
        assert_eq!(
            parse_target("box-123"),
            Some(Holder {
                host: "box".into(),
                pid: 123
            })
        );
        assert_eq!(
            parse_target("my-box-9"),
            Some(Holder {
                host: "my-box".into(),
                pid: 9
            })
        );
        for bad in ["", "box", "box-", "-5", "box-x", "box-0", "box-12x"] {
            assert_eq!(parse_target(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn no_lock_is_free() {
        let t = Tmp::new("free");
        assert_eq!(inspect(&t.profile(), &t.proc(), "my-box"), Lock::Free);
        assert_eq!(check(&t.profile(), &t.proc()), Ok(None));
    }

    #[test]
    fn a_dead_holder_is_stale_and_removed_by_name() {
        let t = Tmp::new("stale");
        t.lock("my-box-4242");
        symlink("0", t.profile().join("SingletonCookie")).unwrap();
        std::fs::write(t.profile().join("keep.txt"), "x").unwrap();
        assert_eq!(
            inspect(&t.profile(), &t.proc(), "my-box"),
            Lock::Stale {
                holder: Holder {
                    host: "my-box".into(),
                    pid: 4242
                }
            }
        );
        let msg = check(&t.profile(), &t.proc()).unwrap().unwrap();
        assert!(msg.contains("4242") && msg.contains("SingletonLock"), "{msg}");
        assert!(std::fs::symlink_metadata(t.profile().join("SingletonLock")).is_err());
        assert!(std::fs::symlink_metadata(t.profile().join("SingletonCookie")).is_err());
        assert!(t.profile().join("keep.txt").exists());
    }

    #[test]
    fn a_live_holder_is_an_error_naming_the_profile_and_pid_and_stays() {
        let t = Tmp::new("live");
        t.lock("my-box-777");
        std::fs::create_dir_all(t.proc().join("777")).unwrap();
        std::fs::write(t.proc().join("777/comm"), "web-host-cef\n").unwrap();
        let err = check(&t.profile(), &t.proc()).unwrap_err();
        assert!(
            err.contains("777") && err.contains("web-host-cef") && err.contains(&t.profile().display().to_string()),
            "{err}"
        );
        assert!(
            std::fs::symlink_metadata(t.profile().join("SingletonLock")).is_ok(),
            "a live lock is never removed"
        );
    }

    #[test]
    fn a_foreign_host_or_odd_target_is_an_error_and_stays() {
        let t = Tmp::new("foreign");
        t.lock("other-box-5");
        let err = check(&t.profile(), &t.proc()).unwrap_err();
        assert!(err.contains("other-box") && err.contains("5"), "{err}");
        assert!(std::fs::symlink_metadata(t.profile().join("SingletonLock")).is_ok());
        std::fs::remove_file(t.profile().join("SingletonLock")).unwrap();
        t.lock("garbage");
        assert!(check(&t.profile(), &t.proc()).unwrap_err().contains("garbage"));
        assert!(std::fs::symlink_metadata(t.profile().join("SingletonLock")).is_ok());
    }
}
