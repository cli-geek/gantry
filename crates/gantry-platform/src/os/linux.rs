use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use keyring_core::CredentialStore;

use super::unix;
use crate::{BrowserInfo, PlatformError, ScheduleSpec, ScheduleStatus};

pub(crate) use unix::{replace_file, restrict_dir, restrict_file, sync_dir};

pub(crate) const NAME: &str = "Linux";

pub(crate) fn credential_store() -> Result<Arc<CredentialStore>, PlatformError> {
    let store: Arc<CredentialStore> = zbus_secret_service_keyring_store::Store::new()?;
    Ok(store)
}

/// Executables searched in `$PATH`, in preference order. All speak CDP.
const BROWSERS: &[&str] = &[
    "chromium",
    "chromium-browser",
    "google-chrome-stable",
    "google-chrome",
    "brave-browser",
    "brave",
    "microsoft-edge-stable",
];

pub(crate) fn locate_browser() -> Result<Option<BrowserInfo>, PlatformError> {
    Ok(BROWSERS.iter().find_map(|name| {
        find_in_path(name).map(|path| BrowserInfo {
            name: (*name).to_owned(),
            path,
        })
    }))
}

const TIMER: &str = "gantry-run.timer";
const SERVICE: &str = "gantry-run.service";

fn unit_dir() -> Result<PathBuf, PlatformError> {
    let base = directories::BaseDirs::new().ok_or(PlatformError::NoHomeDir)?;
    Ok(base.config_dir().join("systemd/user"))
}

pub(crate) fn schedule_install(spec: &ScheduleSpec) -> Result<(), PlatformError> {
    let dir = unit_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| PlatformError::io(&dir, e))?;
    crate::write_atomic(&dir.join(SERVICE), service_unit(spec).as_bytes())?;
    crate::write_atomic(&dir.join(TIMER), timer_unit(spec).as_bytes())?;
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", "--now", TIMER]).map(drop)
}

pub(crate) fn schedule_remove() -> Result<(), PlatformError> {
    let dir = unit_dir()?;
    if dir.join(TIMER).exists() {
        systemctl(&["disable", "--now", TIMER])?;
    }
    for name in [TIMER, SERVICE] {
        let path = dir.join(name);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(PlatformError::io(path, e)),
        }
    }
    systemctl(&["daemon-reload"]).map(drop)
}

pub(crate) fn schedule_status() -> Result<ScheduleStatus, PlatformError> {
    if !unit_dir()?.join(TIMER).exists() {
        return Ok(ScheduleStatus::NotInstalled);
    }
    if find_in_path("systemctl").is_none() {
        return Ok(ScheduleStatus::Unavailable(
            "systemctl not found; the timer unit exists but systemd is not available".into(),
        ));
    }
    // `is-enabled`/`is-active` exit non-zero for "disabled"/"inactive"; the
    // printed state is what matters, so the exit code is ignored here.
    let state = |verb: &str| -> Result<String, PlatformError> {
        let out = Command::new("systemctl")
            .args(["--user", verb, TIMER])
            .output()
            .map_err(|e| PlatformError::io("systemctl", e))?;
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    };
    let enabled = state("is-enabled")?;
    let active = state("is-active")?;
    Ok(ScheduleStatus::Installed {
        enabled: enabled == "enabled",
        active: active == "active",
        detail: format!("systemd user timer {TIMER}: {enabled}, {active}"),
    })
}

fn systemctl(args: &[&str]) -> Result<String, PlatformError> {
    let out = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()
        .map_err(|e| PlatformError::io("systemctl", e))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(PlatformError::Command {
            command: format!("systemctl --user {}", args.join(" ")),
            detail: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        })
    }
}

fn service_unit(spec: &ScheduleSpec) -> String {
    let mut exec = systemd_quote(&spec.program.to_string_lossy());
    for arg in &spec.args {
        exec.push(' ');
        exec.push_str(&systemd_quote(arg));
    }
    format!(
        "[Unit]\nDescription=Gantry nightly run\n\n\
         [Service]\nType=oneshot\nExecStart={exec}\n"
    )
}

fn timer_unit(spec: &ScheduleSpec) -> String {
    format!(
        "[Unit]\nDescription=Gantry nightly run\n\n\
         [Timer]\nOnCalendar=*-*-* {:02}:{:02}:00\nPersistent=true\n\n\
         [Install]\nWantedBy=timers.target\n",
        spec.hour, spec.minute
    )
}

/// Quotes one `ExecStart=` word. systemd expands `%` specifiers and `$`
/// variables even inside quotes, so both are doubled.
fn systemd_quote(word: &str) -> String {
    let mut out = String::with_capacity(word.len() + 2);
    out.push('"');
    for c in word.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '%' => out.push_str("%%"),
            '$' => out.push_str("$$"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Finds `name` in `$PATH` as an executable regular file.
fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| {
            candidate
                .metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_start_quotes_specifiers_and_spaces() {
        let spec = ScheduleSpec {
            program: PathBuf::from("/home/a b/bin/gantry"),
            args: vec!["run".into(), "100%".into(), "$HOME".into()],
            hour: 2,
            minute: 5,
        };
        let unit = service_unit(&spec);
        assert!(unit.contains(r#"ExecStart="/home/a b/bin/gantry" "run" "100%%" "$$HOME""#));
        assert!(timer_unit(&spec).contains("OnCalendar=*-*-* 02:05:00\nPersistent=true"));
    }
}
