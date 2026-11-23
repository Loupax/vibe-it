//! Downloads (from the spike's W5, branch web/w5-streaming). Without a
//! `DownloadHandler` CEF refuses every download (`can_download` defaults to 0). This
//! one saves each download under `--downloads <dir>` (default `<profile>/downloads`)
//! with no prompt and logs its progress and end state. A download UI and the user's
//! download directory are later items.

use cef::*;
use std::sync::{Mutex, OnceLock};

/// Set once in main from `--downloads` / the profile.
pub static DIR: OnceLock<String> = OnceLock::new();

fn dir() -> String {
    DIR.get().cloned().unwrap_or_else(|| "downloads".into())
}

/// Last logged tenth per download, so a 150 MB file logs ~10 lines, not thousands.
static LAST_TENTH: Mutex<Vec<(u32, i32)>> = Mutex::new(Vec::new());

wrap_download_handler! {
    pub struct HostDownloadHandler {}

    impl DownloadHandler {
        fn can_download(&self, _browser: Option<&mut Browser>, url: Option<&CefString>, method: Option<&CefString>) -> i32 {
            log!(
                "download allowed: {} {}",
                method.map(|m| m.to_string()).unwrap_or_default(),
                url.map(|u| u.to_string()).unwrap_or_default()
            );
            1
        }

        fn on_before_download(
            &self,
            _browser: Option<&mut Browser>,
            item: Option<&mut DownloadItem>,
            suggested_name: Option<&CefString>,
            callback: Option<&mut BeforeDownloadCallback>,
        ) -> i32 {
            let (Some(item), Some(callback)) = (item, callback) else { return 0 };
            // Only the final path component: the name is server-controlled.
            let name = suggested_name.map(|n| n.to_string()).unwrap_or_default();
            let name = name.rsplit('/').next().filter(|n| !n.is_empty() && *n != "..").unwrap_or("download.bin").to_string();
            let d = dir();
            let _ = std::fs::create_dir_all(&d);
            let path = format!("{d}/{name}");
            log!(
                "download #{} start: {} -> {path} ({} bytes announced, type {})",
                item.id(),
                CefString::from(&item.url()),
                item.total_bytes(),
                CefString::from(&item.mime_type())
            );
            callback.cont(Some(&CefString::from(path.as_str())), 0); // 0: no dialog
            1
        }

        fn on_download_updated(&self, _browser: Option<&mut Browser>, item: Option<&mut DownloadItem>, _callback: Option<&mut DownloadItemCallback>) {
            let Some(item) = item else { return };
            let state = if item.is_complete() != 0 {
                "COMPLETE"
            } else if item.is_canceled() != 0 {
                "CANCELED"
            } else if item.is_interrupted() != 0 {
                "INTERRUPTED"
            } else {
                "in progress"
            };
            let pct = item.percent_complete();
            let id = item.id();
            if state == "in progress" {
                let mut last = LAST_TENTH.lock().unwrap();
                let step = pct / 10;
                match last.iter_mut().find(|(i, _)| *i == id) {
                    Some((_, p)) if *p == step => return,
                    Some((_, p)) => *p = step,
                    None => last.push((id, step)),
                }
            }
            let reason = if item.is_interrupted() != 0 {
                format!(", reason {:?}", sys::cef_download_interrupt_reason_t::from(item.interrupt_reason()))
            } else {
                String::new()
            };
            log!(
                "download #{id} {state}: {} of {} bytes ({pct}%), {} B/s{reason}",
                item.received_bytes(),
                item.total_bytes(),
                item.current_speed()
            );
        }
    }
}
