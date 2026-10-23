// The shell-level pieces: the panic hook that survives a silent crash, and the
// handle the hook uses to show the crash dialog.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri_plugin_dialog::DialogExt;

use crate::paths::data_dir;

// The shell-level pieces: the panic hook that survives a silent crash, and the
// handle the hook uses to show the crash dialog.

/// True once a panic unwound through the process; gates the crash-report sleep
/// after the event loop stops.
pub(crate) static PANICKED: AtomicBool = AtomicBool::new(false);

/// Set as soon as the app exists so the panic hook can open a dialog.
pub(crate) static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

pub(crate) fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        PANICKED.store(true, Ordering::SeqCst);
        let text = format!(
            "--- panic at {} ---\n{}\n{}\n",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            info,
            std::backtrace::Backtrace::force_capture()
        );
        let log = data_dir().join("panic.log");
        if let Some(parent) = log.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
        {
            use std::io::Write;
            let _ = writeln!(file, "{text}");
        }
        if let Some(app) = APP_HANDLE.get().cloned() {
            thread::spawn(move || {
                thread::sleep(std::time::Duration::from_millis(200));
                app.dialog().message(format!("Local-first AI Video Studio hit an internal error and had to stop.\n\nDetails were saved to:\n{}", log.display())).title("Unexpected error").blocking_show();
            });
        }
    }));
}

