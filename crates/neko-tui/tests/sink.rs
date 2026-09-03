//! A stand-in clipboard backend that records what it was handed.
//!
//! Tests must not overwrite the developer's real clipboard, and asserting that
//! *some* message appeared is not enough - the bug this guards against was a
//! backend that silently did nothing while the UI reported success. So the
//! copy is routed to a file and the payload is checked.
//!
//! The production backend pipes text to a program's stdin and discards its
//! stdout, so the recorder has to write the file itself rather than rely on a
//! shell redirect.

#![allow(dead_code)]

use std::path::Path;

use neko_tui::clipboard::Clipboard;

/// A backend that writes whatever it receives to `path` and exits 0.
pub fn recorder(path: &Path) -> Clipboard {
    #[cfg(unix)]
    {
        Clipboard::Native {
            program: "/usr/bin/tee".into(),
            args: vec![path.to_string_lossy().to_string()],
        }
    }
    #[cfg(windows)]
    {
        // PowerShell writes the file itself, so no redirect has to survive
        // Rust's argument quoting. `ascii` avoids the byte-order mark that
        // `Set-Content` writes by default, which would break an exact compare.
        Clipboard::Native {
            program: "powershell".into(),
            args: vec![
                "-NoProfile".into(),
                "-Command".into(),
                format!(
                    "$input | Set-Content -LiteralPath '{}' -Encoding ascii -NoNewline",
                    path.display()
                ),
            ],
        }
    }
}

/// A backend that cannot possibly run.
pub fn broken() -> Clipboard {
    Clipboard::Native {
        program: "/nonexistent/clipboard-helper".into(),
        args: vec![],
    }
}

/// What the recorder captured. Trimmed: a helper may add a line ending of its
/// own, which says nothing about whether the payload arrived.
pub fn captured(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_string()
}
