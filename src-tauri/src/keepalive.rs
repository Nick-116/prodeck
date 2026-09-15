//! "Keep ProDeck running": start at login, relaunch after a crash, and don't
//! let the machine fall asleep mid-service. A booth computer has to survive
//! unattended; the DMG install used to get none of this.
//!
//! Three platform implementations behind one API:
//!   * macOS — a per-user LaunchAgent plus `caffeinate` bound to our own pid.
//!   * Linux — a systemd user service (start-at-login + crash-relaunch) plus
//!             `systemd-inhibit` bound to our own lifetime for sleep prevention.
//!   * Windows — an HKCU Run entry plus SetThreadExecutionState.
//!
//! ⚠️ The Windows path has never been compiled or run. Cross-compiling from
//! macOS stops at a C dependency that needs the MSVC toolchain, so it cannot
//! even be type-checked here. Treat it as a starting point that needs a
//! Windows machine, not as working code. macOS and Linux are unaffected either
//! way: every platform call is behind #[cfg].

use serde_json::{json, Value};
use std::sync::Mutex;
use crate::app::AppHandle;

pub const LABEL: &str = "com.prodeck.watchdog";

/// What the sleep guard holds onto. macOS and Linux keep the inhibitor child so
/// it dies with us; Windows just flips a thread flag, so there is nothing to hold.
#[cfg(target_os = "macos")]
pub type AwakeGuard = std::process::Child;
/// Windows keeps the sender for the thread that holds the execution-state flag:
/// the flag is per-THREAD and Windows drops it the moment that thread ends, so
/// the request has to be owned by a thread that stays alive. Dropping this
/// sender is what tells that thread to release it and exit.
#[cfg(windows)]
pub type AwakeGuard = std::sync::mpsc::Sender<()>;
/// Linux: holds the `systemd-inhibit … sleep infinity` child — same
/// "nothing left behind" property as caffeinate: it dies when we do.
#[cfg(not(any(target_os = "macos", windows)))]
pub type AwakeGuard = std::process::Child;

pub struct KeepAwake(pub Mutex<Option<AwakeGuard>>);

fn current_exe() -> String {
    std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()
}

// ===========================================================================
// macOS
// ===========================================================================
#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    /// Where a booth install must live for the watchdog to have a stable path.
    pub const INSTALL_HINT: &str = "Move ProDeck to your Applications folder first — the watchdog needs a permanent path to relaunch.";

    pub fn in_install_dir(exe: &str) -> bool {
        exe.starts_with("/Applications/")
    }

    pub fn under_supervisor() -> bool {
        std::os::unix::process::parent_id() == 1
    }

    fn plist_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join("Library/LaunchAgents").join(format!("{LABEL}.plist")))
    }

    fn uid() -> String {
        String::from_utf8_lossy(
            &Command::new("/usr/bin/id").arg("-u").output().map(|o| o.stdout).unwrap_or_default(),
        )
        .trim()
        .to_string()
    }

    fn launchctl(args: &[&str]) -> Result<(), String> {
        let out = Command::new("/bin/launchctl").args(args).output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }

    /// The program the installed job points at, if any.
    pub fn installed_program() -> Option<String> {
        let txt = std::fs::read_to_string(plist_path()?).ok()?;
        let i = txt.find("<key>ProgramArguments</key>")?;
        let rest = &txt[i..];
        let s = rest.find("<string>")? + "<string>".len();
        let e = rest[s..].find("</string>")? + s;
        Some(rest[s..e].to_string())
    }

    pub fn install(exe: &str) -> Result<(), String> {
        let path = plist_path().ok_or("no home directory")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <!-- Written by ProDeck (Settings → Reliability). Starts ProDeck at login and
       relaunches it within ~10s of any crash. A deliberate Quit stays quit. -->
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array><string>{exe}</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ThrottleInterval</key><integer>10</integer>
</dict>
</plist>
"#
        );
        std::fs::write(&path, plist).map_err(|e| e.to_string())?;
        if !under_supervisor() {
            let domain = format!("gui/{}", uid());
            let p = path.to_str().unwrap_or("");
            let _ = launchctl(&["bootout", &domain, p]);
            launchctl(&["bootstrap", &domain, p])
                .or_else(|e| if e.contains("already") { Ok(()) } else { Err(e) })?;
            // RunAtLoad started a second copy; keep the one the user is looking at.
            kill_other_instances();
        }
        Ok(())
    }

    fn kill_other_instances() {
        let me = std::process::id().to_string();
        if let Ok(out) = Command::new("/usr/bin/pgrep").args(["-x", "prodeck"]).output() {
            for pid in String::from_utf8_lossy(&out.stdout).split_whitespace() {
                if pid != me {
                    let _ = Command::new("/bin/kill").args(["-TERM", pid]).output();
                }
            }
        }
    }

    pub fn uninstall() -> Result<(), String> {
        let path = plist_path().ok_or("no home directory")?;
        let _ = launchctl(&["bootout", &format!("gui/{}/{LABEL}", uid())]);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn relaunch() -> Result<(), String> {
        let path = plist_path().ok_or("no home directory")?;
        if !path.exists() {
            return Err("Turn on Keep ProDeck running first.".into());
        }
        let domain = format!("gui/{}", uid());
        let _ = launchctl(&["bootstrap", &domain, path.to_str().unwrap_or("")]);
        launchctl(&["kickstart", "-k", &format!("{domain}/{LABEL}")])
    }

    /// `caffeinate -is -w <our pid>` blocks idle and system sleep for exactly as
    /// long as this process lives, and dies with it — nothing left behind.
    /// Display sleep is deliberately still allowed; a booth screen may dim.
    pub fn wake_on(slot: &mut Option<AwakeGuard>) {
        if slot.is_some() {
            return;
        }
        match Command::new("/usr/bin/caffeinate")
            .args(["-is", "-w", &std::process::id().to_string()])
            .spawn()
        {
            Ok(child) => {
                *slot = Some(child);
                crate::diag::log("[keepalive] sleep guard on");
            }
            Err(e) => crate::diag::log(format!("[keepalive] caffeinate failed: {e}")),
        }
    }

    pub fn wake_off(slot: &mut Option<AwakeGuard>) {
        if let Some(mut child) = slot.take() {
            let _ = child.kill();
            let _ = child.wait();
            crate::diag::log("[keepalive] sleep guard off");
        }
    }
}

// ===========================================================================
// Windows  — WRITTEN BUT NEVER COMPILED. See the module note above.
// ===========================================================================
#[cfg(windows)]
mod platform {
    use super::*;

    pub const INSTALL_HINT: &str =
        "Move ProDeck to Program Files (or another permanent folder) first — the watchdog needs a path that won't move.";

    /// On Windows there is no single blessed install directory the way
    /// /Applications is on macOS, so the only thing that actually matters is
    /// that the path is stable. Downloads and temp folders are not.
    pub fn in_install_dir(exe: &str) -> bool {
        let low = exe.to_ascii_lowercase();
        // `target\debug` and `target\release` matter as much as Downloads: a
        // dev build that registers itself points the Run key at a path
        // `cargo clean` deletes, and the machine then fails to start ProDeck
        // at login forever after.
        !(low.contains("\\downloads\\")
            || low.contains("\\temp\\")
            || low.contains("\\appdata\\local\\temp")
            || low.contains("\\target\\debug\\")
            || low.contains("\\target\\release\\"))
    }

    /// Windows has no launchd equivalent that supervises a GUI app, so a Run
    /// entry gives start-at-login but NOT crash-relaunch. The UI is told this
    /// via `supervises` so it can't promise something Windows won't do.
    pub fn under_supervisor() -> bool {
        false
    }

    const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

    fn reg(args: &[&str]) -> Result<String, String> {
        // Full path, not PATH lookup, and no console window — the Reliability
        // panel calls this on open and after every change, and each spawn from
        // a GUI process otherwise flashes a black cmd window.
        let exe = std::env::var("SystemRoot")
            .map(|r| format!(r"{r}\System32\reg.exe"))
            .unwrap_or_else(|_| "reg".into());
        let out = crate::diag::command(&exe).args(args).output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).to_string())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }

    pub fn installed_program() -> Option<String> {
        let out = reg(&["query", RUN_KEY, "/v", "ProDeck"]).ok()?;
        // `reg query` prints:  ProDeck    REG_SZ    "C:\...\ProDeck.exe"
        let line = out.lines().find(|l| l.contains("ProDeck"))?;
        // Split on the type column rather than the literal "REG_SZ": a value
        // written as REG_EXPAND_SZ (by anything other than us) does not contain
        // "REG_SZ" as a substring, and the old split reported "not installed"
        // while leaving an entry the UI then couldn't remove.
        let val = line
            .split_once("REG_EXPAND_SZ")
            .or_else(|| line.split_once("REG_SZ"))
            .map(|(_, rest)| rest)?
            .trim();
        Some(val.trim_matches('"').to_string())
    }

    pub fn install(exe: &str) -> Result<(), String> {
        reg(&["add", RUN_KEY, "/v", "ProDeck", "/t", "REG_SZ", "/d", exe, "/f"]).map(|_| ())
    }

    pub fn uninstall() -> Result<(), String> {
        match reg(&["delete", RUN_KEY, "/v", "ProDeck", "/f"]) {
            Ok(_) => Ok(()),
            // Deleting something that isn't there is the desired end state.
            Err(e) if e.to_lowercase().contains("unable to find") => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub fn relaunch() -> Result<(), String> {
        Err("On Windows, quit and reopen ProDeck to restart it.".into())
    }

    // SetThreadExecutionState keeps the machine awake for as long as the thread
    // that called it is alive, and Windows clears it when the process exits —
    // the same "nothing left behind" property as caffeinate.
    //
    // "as long as the THREAD is alive" is the whole difficulty. Setting the flag
    // from the command handler would tie the booth's sleep guard to whatever
    // thread Tauri happened to dispatch that call on: the moment it returned to
    // the pool the request would be dropped, the machine would sleep mid-service,
    // and the UI would still be showing "sleep guard on". So the flag is owned
    // by a thread of our own that does nothing but hold it and wait to be told
    // to stop. Verify on the machine with `powercfg /requests`: the SYSTEM
    // section must name ProDeck.exe.
    const ES_CONTINUOUS: u32 = 0x8000_0000;
    const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;

    #[link(name = "kernel32")]
    extern "system" {
        fn SetThreadExecutionState(flags: u32) -> u32;
    }

    pub fn wake_on(slot: &mut Option<AwakeGuard>) {
        if slot.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        std::thread::Builder::new()
            .name("prodeck-keepawake".into())
            .spawn(move || {
                // SAFETY: a documented kernel32 call taking a bitflag and
                // returning the previous state. No pointers, no allocation.
                unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
                // Blocks until the sender is dropped (wake_off, or shutdown).
                // The return value is deliberately ignored: either message or
                // disconnect means "release it".
                let _ = rx.recv();
                // SAFETY: as above — clears our request, restoring normal sleep.
                unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
            })
            .ok();
        // The previous-state return is NOT checked for zero. Zero is a plausible
        // answer for a thread that has never called it, and treating that as
        // failure meant the slot was never filled, so the toggle could never
        // latch on however many times it was pressed.
        *slot = Some(tx);
        crate::diag::log("[keepalive] sleep guard on");
    }

    pub fn wake_off(slot: &mut Option<AwakeGuard>) {
        if slot.take().is_some() {
            // Dropping the sender wakes the holder thread, which clears the
            // flag and exits.
            crate::diag::log("[keepalive] sleep guard off");
        }
    }
}

// ===========================================================================
// Linux — systemd user service (watchdog + start-at-login) + systemd-inhibit
// (sleep guard).
// ===========================================================================
#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    pub const INSTALL_HINT: &str =
        "Move ProDeck to /usr/local/bin or another permanent directory first — the watchdog needs a stable path to relaunch.";

    pub fn in_install_dir(exe: &str) -> bool {
        !(exe.contains("/target/debug/")
            || exe.contains("/target/release/")
            || exe.starts_with("/tmp/")
            || exe.starts_with("/var/tmp/"))
    }

    /// systemd sets INVOCATION_ID on every service it manages — it is the
    /// canonical way to detect "running under systemd" from inside a process.
    pub fn under_supervisor() -> bool {
        std::env::var("INVOCATION_ID").is_ok()
    }

    fn autostart_path() -> Option<PathBuf> {
        dirs::config_dir().map(|c| c.join("autostart/prodeck.desktop"))
    }

    fn service_path() -> Option<PathBuf> {
        dirs::config_dir().map(|c| c.join("systemd/user/prodeck.service"))
    }

    fn systemctl(args: &[&str]) -> Result<(), String> {
        let out = Command::new("systemctl")
            .args(args)
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }

    /// The exe path from the installed unit file, if any.
    pub fn installed_program() -> Option<String> {
        let txt = std::fs::read_to_string(service_path()?).ok()?;
        txt.lines()
            .find(|l| l.starts_with("ExecStart="))?
            .strip_prefix("ExecStart=")
            .map(|s| s.trim().to_string())
    }

    pub fn install(exe: &str) -> Result<(), String> {
        // XDG autostart entry — for desktop environments that respect it.
        if let Some(path) = autostart_path() {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            let desktop = format!(
                "[Desktop Entry]\nType=Application\nName=ProDeck\nExec={exe}\nHidden=false\nNoDisplay=false\nX-GNOME-Autostart-enabled=true\n"
            );
            std::fs::write(&path, desktop).map_err(|e| e.to_string())?;
        }

        // systemd user service — gives crash-relaunch on top of start-at-login.
        if let Some(path) = service_path() {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            let unit = format!(
                "[Unit]\nDescription=ProDeck booth hub\nAfter=network.target\n\n[Service]\nExecStart={exe}\nRestart=on-failure\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n"
            );
            std::fs::write(&path, unit).map_err(|e| e.to_string())?;
            let _ = systemctl(&["--user", "daemon-reload"]);
            systemctl(&["--user", "enable", "prodeck"])
                .or_else(|e| if e.contains("already") { Ok(()) } else { Err(e) })?;
            kill_other_instances();
        }

        Ok(())
    }

    fn kill_other_instances() {
        let me = std::process::id().to_string();
        if let Ok(out) = Command::new("pgrep").args(["-x", "prodeck"]).output() {
            for pid in String::from_utf8_lossy(&out.stdout).split_whitespace() {
                if pid != me {
                    let _ = Command::new("kill").args(["-TERM", pid]).output();
                }
            }
        }
    }

    pub fn uninstall() -> Result<(), String> {
        let _ = systemctl(&["--user", "disable", "prodeck"]);
        if let Some(path) = service_path() {
            if path.exists() {
                std::fs::remove_file(&path).map_err(|e| e.to_string())?;
            }
        }
        let _ = systemctl(&["--user", "daemon-reload"]);
        if let Some(path) = autostart_path() {
            if path.exists() {
                std::fs::remove_file(&path).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    pub fn relaunch() -> Result<(), String> {
        if service_path().filter(|p| p.exists()).is_none() {
            return Err("Turn on Keep ProDeck running first.".into());
        }
        systemctl(&["--user", "restart", "prodeck"])
    }

    /// `systemd-inhibit --what=sleep:idle --mode=block sleep infinity` blocks
    /// idle and system sleep for exactly as long as the child lives — nothing
    /// left behind, identical "dies with us" property to caffeinate on macOS.
    pub fn wake_on(slot: &mut Option<AwakeGuard>) {
        if slot.is_some() {
            return;
        }
        match Command::new("systemd-inhibit")
            .args([
                "--what=sleep:idle",
                "--who=ProDeck",
                "--why=Booth computer",
                "--mode=block",
                "sleep",
                "infinity",
            ])
            .spawn()
        {
            Ok(child) => {
                *slot = Some(child);
                crate::diag::log("[keepalive] sleep guard on");
            }
            Err(e) => crate::diag::log(format!("[keepalive] systemd-inhibit failed: {e}")),
        }
    }

    pub fn wake_off(slot: &mut Option<AwakeGuard>) {
        if let Some(mut child) = slot.take() {
            let _ = child.kill();
            let _ = child.wait();
            crate::diag::log("[keepalive] sleep guard off");
        }
    }
}

// ===========================================================================
// Shared API
// ===========================================================================

pub fn status_value(app: &AppHandle) -> Value {
    let exe = current_exe();
    let installed = platform::installed_program();
    let awake = app
        .try_state::<KeepAwake>()
        .map(|k| k.0.lock().unwrap_or_else(|p| p.into_inner()).is_some())
        .unwrap_or(false);
    json!({
        "installed": installed.is_some(),
        "program": installed,
        "matchesCurrent": installed.as_deref() == Some(exe.as_str()),
        "underLaunchd": platform::under_supervisor(),
        "inApplications": platform::in_install_dir(&exe),
        "exe": exe,
        "keepAwake": awake,
        // Whether this platform can relaunch after a CRASH, or only start at
        // login. macOS (launchd) and Linux (systemd) can; a Windows Run key
        // cannot, and the UI must not claim otherwise.
        "supervises": cfg!(not(windows)),
        "installHint": platform::INSTALL_HINT,
    })
}

pub fn keepalive_status(app: &AppHandle) -> Value {
    status_value(app)
}

/// Dispatch-facing alias for status.
pub fn status_core(app: &AppHandle) -> Value {
    status_value(app)
}

/// Not available in Docker mode — use Docker/systemd for process management.
pub fn keepalive_install(_app: &AppHandle) -> Result<Value, String> {
    Err("Use Docker or your system's process manager for ProDeck lifecycle management".into())
}

/// Not available in Docker mode — use Docker/systemd for process management.
pub fn keepalive_uninstall(_app: &AppHandle) -> Result<Value, String> {
    Err("Use Docker or your system's process manager for ProDeck lifecycle management".into())
}

/// Not available in Docker mode — use Docker/systemd for process management.
pub fn keepalive_relaunch(_app: &AppHandle) -> Result<(), String> {
    Err("Use Docker or your system's process manager to restart ProDeck".into())
}

pub fn set_keep_awake(app: &AppHandle, on: bool) {
    let Some(state) = app.try_state::<KeepAwake>() else { return };
    let mut g = state.0.lock().unwrap_or_else(|p| p.into_inner());
    if on {
        platform::wake_on(&mut g);
    } else {
        platform::wake_off(&mut g);
    }
}

pub fn keep_awake_set(on: bool, app: &AppHandle) -> Result<Value, String> {
    set_keep_awake(app, on);
    {
        let st = app.state::<crate::settings::SettingsState>();
        let to_save = {
            let mut s = st.lock().unwrap_or_else(|p| p.into_inner());
            s.keep_awake = on;
            s.clone()
        };
        crate::settings::save(&to_save)?;
    }
    Ok(status_value(app))
}
