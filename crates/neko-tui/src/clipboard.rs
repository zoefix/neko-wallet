//! Copying to the clipboard.
//!
//! Three backends, chosen at runtime, in a deliberate order of preference:
//!
//! 1. **A native helper** (`pbcopy`, `wl-copy`, `xclip`, `clip`). The only one
//!    whose result we can actually observe: the helper exits with a status, so
//!    "copied" is a fact rather than a hope. These also survive the wallet
//!    exiting, because each of them either hands the data to a system service
//!    (macOS, Windows) or daemonizes to serve the X/Wayland selection.
//! 2. **OSC 52**, for SSH sessions, where the native clipboard would be the
//!    *server's* and therefore useless. The terminal never replies, so success
//!    can never be claimed - only that the request was sent.
//! 3. **Nothing**, said plainly, so the user reaches for the mouse instead of
//!    pasting whatever was in the clipboard beforehand.
//!
//! Preferring OSC 52 when a native helper exists is a mistake worth naming: it
//! was the original bug here. `detect` returned `Osc52` unconditionally, and on
//! macOS Terminal.app - which does not implement OSC 52 - the escape sequence
//! was swallowed in silence while the UI reported that the copy had been sent.
//! Pasting then produced whatever was on the clipboard before, which for a
//! wallet address is precisely the failure that loses money.

use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clipboard {
    /// An external helper that takes the text on stdin.
    Native { program: PathBuf, args: Vec<String> },
    /// Escape sequence handed to the terminal. Fire-and-forget.
    Osc52,
    /// No usable path; the UI shows the text for manual selection instead.
    Unavailable,
}

/// What the user should be told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyOutcome {
    /// We know it landed.
    Confirmed,
    /// We asked the terminal and cannot know whether it complied. The message
    /// shown to the user must reflect that rather than claiming success.
    Sent,
}

impl Clipboard {
    pub fn detect() -> Self {
        // Over SSH the native helper - if there even is one - would set the
        // clipboard of the machine the user is not sitting at.
        if Self::is_remote_session() {
            return Clipboard::Osc52;
        }
        match native_helper() {
            Some((program, args)) => Clipboard::Native { program, args },
            // No helper installed. Many terminals still honour OSC 52, and an
            // unverifiable attempt beats refusing outright.
            None => Clipboard::Osc52,
        }
    }

    pub fn is_remote_session() -> bool {
        ["SSH_CONNECTION", "SSH_TTY", "SSH_CLIENT"]
            .iter()
            .any(|k| std::env::var_os(k).is_some())
    }

    pub fn copy(&self, text: &str) -> Option<CopyOutcome> {
        match self {
            Clipboard::Unavailable => None,
            Clipboard::Native { program, args } => {
                // Text goes in on stdin, never as an argument: no shell is
                // involved, so nothing here can be interpreted as one.
                let mut child = std::process::Command::new(program)
                    .args(args)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .ok()?;
                child.stdin.take()?.write_all(text.as_bytes()).ok()?;
                // A helper that ran but failed must report failure. Reporting
                // success here would be the original bug in a new costume.
                child
                    .wait()
                    .ok()?
                    .success()
                    .then_some(CopyOutcome::Confirmed)
            }
            Clipboard::Osc52 => {
                let payload = base64(text.as_bytes());
                let seq = format!("\x1b]52;c;{payload}\x07");
                // tmux and screen swallow unknown OSC sequences unless they are
                // wrapped in a DCS passthrough.
                let wrapped = if std::env::var_os("TMUX").is_some() {
                    format!("\x1bPtmux;{}\x1b\\", seq.replace('\x1b', "\x1b\x1b"))
                } else {
                    seq
                };
                let mut out = std::io::stdout();
                out.write_all(wrapped.as_bytes()).ok()?;
                out.flush().ok()?;
                Some(CopyOutcome::Sent)
            }
        }
    }

    /// Whether a successful copy can be confirmed rather than merely attempted.
    /// The UI wording depends on this.
    pub fn is_verifiable(&self) -> bool {
        matches!(self, Clipboard::Native { .. })
    }
}

/// The first clipboard helper on this system that actually exists.
fn native_helper() -> Option<(PathBuf, Vec<String>)> {
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else if cfg!(windows) {
        &[("clip", &[])]
    } else {
        &[
            // Wayland first: on a Wayland session xclip may exist but only
            // reach a nested XWayland clipboard.
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ]
    };
    for (program, args) in candidates {
        if let Some(path) = which(program) {
            return Some((path, args.iter().map(|s| s.to_string()).collect()));
        }
    }
    None
}

/// Resolve a program on `PATH`.
///
/// Done by hand rather than by spawning the command to see whether it works:
/// the only way to test a clipboard helper is to let it overwrite the
/// clipboard, which is not something to do at startup.
fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    // Windows executables need an extension to be spawnable.
    let names: Vec<String> = if cfg!(windows) {
        vec![format!("{program}.exe"), program.to_string()]
    } else {
        vec![program.to_string()]
    };
    for dir in std::env::split_paths(&path) {
        for name in &names {
            let candidate = dir.join(name);
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn is_executable(p: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        out.push(T[(n >> 18 & 63) as usize] as char);
        out.push(T[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_answers() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        // A real TRON address, the thing this actually copies.
        assert_eq!(
            base64(b"TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t"),
            "VFI3TkhxamVLUXhHVENpOHE4Wlk0cEw4b3RTemdqTGo2dA=="
        );
    }

    /// The regression. On a normal desktop session there is a native helper,
    /// and it must be chosen: OSC 52 is unverifiable and macOS Terminal.app
    /// ignores it entirely.
    #[test]
    fn a_local_session_uses_the_native_helper() {
        if native_helper().is_none() {
            // A headless CI box with no xclip. Nothing to assert.
            return;
        }
        assert!(
            matches!(Clipboard::detect(), Clipboard::Native { .. }),
            "a local session fell back to OSC 52 despite a working helper"
        );
    }

    /// The text must reach the helper's stdin intact, and a successful run
    /// must be reported as confirmed.
    #[cfg(unix)]
    #[test]
    fn text_reaches_the_helper_and_success_is_confirmed() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("clip.txt");
        // `tee` consumes stdin and exits 0, standing in for pbcopy without
        // touching the real clipboard while tests run.
        let Some(tee) = which("tee") else { return };
        let c = Clipboard::Native {
            program: tee,
            args: vec![out.to_string_lossy().to_string()],
        };
        const ADDR: &str = "TPZrDZTUWQqqUTVRxAmSdQyGXSSgAUyyk4";
        assert_eq!(c.copy(ADDR), Some(CopyOutcome::Confirmed));
        assert_eq!(std::fs::read_to_string(&out).unwrap(), ADDR);
        assert!(c.is_verifiable());
    }

    /// A helper that exits non-zero must NOT be reported as a successful copy.
    /// Claiming success when the clipboard still holds the previous value is
    /// how somebody pastes the wrong address.
    #[cfg(unix)]
    #[test]
    fn a_failing_helper_is_not_reported_as_success() {
        let Some(f) = which("false") else { return };
        let c = Clipboard::Native {
            program: f,
            args: vec![],
        };
        assert_eq!(c.copy("anything"), None);
    }

    #[test]
    fn a_missing_helper_is_not_reported_as_success() {
        let c = Clipboard::Native {
            program: PathBuf::from("/nonexistent/no-such-clipboard-helper"),
            args: vec![],
        };
        assert_eq!(c.copy("anything"), None);
        assert_eq!(Clipboard::Unavailable.copy("anything"), None);
    }

    /// OSC 52 cannot be confirmed, and must never claim it was.
    #[test]
    fn osc52_is_never_reported_as_confirmed() {
        assert!(!Clipboard::Osc52.is_verifiable());
        assert_ne!(
            Clipboard::Osc52.copy("x"),
            Some(CopyOutcome::Confirmed),
            "OSC 52 claimed a success it cannot possibly know about"
        );
    }

    #[test]
    fn which_finds_real_programs_and_rejects_invented_ones() {
        assert!(which("no-such-program-anywhere-12345").is_none());
        #[cfg(unix)]
        assert!(which("sh").is_some(), "sh should be on PATH");
    }
}
