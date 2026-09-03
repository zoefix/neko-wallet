//! neko-wallet: a terminal TRON wallet.

mod config;
mod paths;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "neko-wallet", version, about = "Terminal TRON wallet")]
struct Cli {
    /// Path to the encrypted vault file.
    #[arg(long, value_name = "PATH", env = paths::ENV_VAR)]
    db: Option<PathBuf>,

    /// Print the version in a machine-readable form and exit.
    ///
    /// One stable line, so the install script can confirm what it just put on
    /// disk without parsing clap's prettier `--version` output.
    #[arg(long)]
    machine_readable: bool,

    /// Print the resolved vault path and exit.
    #[arg(long)]
    where_db: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Change a setting that applies before the vault is opened.
    Set {
        #[command(subcommand)]
        what: SetWhat,
    },
    /// Clear a saved setting.
    Unset {
        #[command(subcommand)]
        what: UnsetWhat,
    },
}

#[derive(Subcommand, Debug)]
enum SetWhat {
    /// Remember which vault file to open from now on.
    ///
    /// Needs no email or password: this only records a path, and the vault it
    /// points at stays exactly as encrypted as it was.
    Db {
        /// Path to an existing `.db` vault.
        path: PathBuf,
        /// Accept a path that does not exist yet, to start a new vault there.
        ///
        /// Off by default because the usual reason a path does not exist is a
        /// typo, and the consequence is a first-run screen that looks like the
        /// original wallet has vanished.
        #[arg(long)]
        new: bool,
    },
}

#[derive(Subcommand, Debug)]
enum UnsetWhat {
    /// Forget the saved vault path and go back to the default search.
    Db,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let cli = Cli::parse();

    if let Some(cmd) = cli.command {
        neko_i18n::set_locale(neko_i18n::Locale::detect());
        return run_command(cmd);
    }

    let db = paths::resolve(cli.db);

    if cli.machine_readable {
        println!("neko-wallet {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if cli.where_db {
        println!("{}", db.display());
        return Ok(());
    }
    // A configured vault that has gone missing must be said out loud. Left
    // unsaid, the next screen is first-run setup, which reads as "my wallet is
    // gone" and invites the user to create a replacement over the top.
    let warning = missing_vault_warning(&db);
    neko_tui::run_with(db, warning).await
}

// ── subcommands ────────────────────────────────────────────────────────────

fn run_command(cmd: Command) -> std::io::Result<()> {
    use neko_i18n::{t, tf, Key};
    match cmd {
        Command::Set {
            what: SetWhat::Db { path, new },
        } => {
            // Stored absolute: a relative path would mean something different
            // from every directory the user later happens to be in.
            let abs = absolute(&path)?;

            match config::check_vault(&abs) {
                config::VaultCheck::Ok { profile } => {
                    println!("{}", tf(Key::Cli_DbIsVault, &[("profile", profile)]));
                }
                config::VaultCheck::Missing if new => {
                    println!("{}", t(Key::Cli_DbWillBeCreated));
                }
                config::VaultCheck::Missing => {
                    return fail(tf(
                        Key::Cli_DbMissing,
                        &[("path", &abs.display().to_string())],
                    ));
                }
                config::VaultCheck::NotAVault(why) => {
                    return fail(tf(
                        Key::Cli_DbNotAVault,
                        &[("path", &abs.display().to_string()), ("reason", &why)],
                    ));
                }
            }

            let mut cfg = config::load();
            cfg.db = Some(abs.clone());
            let at = config::save(&cfg)?;
            println!(
                "{}",
                tf(Key::Cli_DbSet, &[("path", &abs.display().to_string())])
            );
            println!(
                "{}",
                tf(Key::Cli_ConfigAt, &[("path", &at.display().to_string())])
            );
            Ok(())
        }
        Command::Unset {
            what: UnsetWhat::Db,
        } => {
            let mut cfg = config::load();
            cfg.db = None;
            config::save(&cfg)?;
            let now = paths::resolve(None);
            println!("{}", t(Key::Cli_DbUnset));
            println!(
                "{}",
                tf(Key::Cli_DbNow, &[("path", &now.display().to_string())])
            );
            Ok(())
        }
    }
}

fn fail(message: String) -> std::io::Result<()> {
    eprintln!("{message}");
    std::process::exit(1);
}

/// Make a path absolute without requiring it to exist, so `--new` works.
fn absolute(p: &std::path::Path) -> std::io::Result<PathBuf> {
    // `canonicalize` resolves symlinks and `..`, but only for paths that
    // already exist; fall back to joining onto the working directory.
    if let Ok(c) = std::fs::canonicalize(p) {
        return Ok(c);
    }
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()?.join(p)
    };
    // The parent usually does exist, which is enough to resolve any `..`.
    match (joined.parent(), joined.file_name()) {
        (Some(dir), Some(name)) => Ok(std::fs::canonicalize(dir)
            .map(|d| d.join(name))
            .unwrap_or(joined.clone())),
        _ => Ok(joined),
    }
}

/// A warning when the vault we were told to open is not there.
fn missing_vault_warning(db: &std::path::Path) -> Option<String> {
    if db.exists() || config::load().db.as_deref() != Some(db) {
        return None;
    }
    Some(neko_i18n::tf(
        neko_i18n::Key::Cli_ConfiguredDbMissing,
        &[("path", &db.display().to_string())],
    ))
}
