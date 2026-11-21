mod model;
mod render;

use anyhow::{bail, Context, Result};
use clap::Parser;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use model::Export;

/// Turn an OpenCode session export into a beautiful standalone HTML page.
#[derive(Debug, Parser)]
#[command(name = "oc-observe", version, about, long_about = None)]
struct Cli {
    /// Session id (ses_...), a path to an exported .json file, or `-` for stdin
    session: String,

    /// Output HTML path [default: <session-id>.html]
    #[arg(short, long, value_name = "FILE", conflicts_with = "stdout")]
    output: Option<PathBuf>,

    /// Print the HTML to stdout instead of writing a file
    #[arg(long)]
    stdout: bool,

    /// Open the generated page in the default browser
    #[arg(long, conflicts_with = "stdout")]
    open: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let export = load_export(&cli.session)?;
    let html = render::render(&export);

    if cli.stdout {
        let mut out = std::io::stdout().lock();
        out.write_all(html.as_bytes())?;
        return Ok(());
    }

    let path = cli
        .output
        .unwrap_or_else(|| PathBuf::from(format!("{}.html", safe_name(&export.info.id))));

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
    }

    fs::write(&path, html.as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))?;
    println!("{}", path.display());

    if cli.open {
        if let Err(err) = open_in_browser(&path) {
            eprintln!("warning: could not open browser: {err}");
        }
    }

    Ok(())
}

fn load_export(arg: &str) -> Result<Export> {
    let raw = if arg == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("failed to read export JSON from stdin")?;
        buf
    } else if arg.ends_with(".json") && Path::new(arg).is_file() {
        fs::read_to_string(arg).with_context(|| format!("failed to read {arg}"))?
    } else {
        run_export(arg)?
    };

    if raw.trim().is_empty() {
        bail!("empty session export data");
    }

    serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse session export JSON for '{arg}'"))
}

fn run_export(id: &str) -> Result<String> {
    let output = Command::new("opencode")
        .args(["session", "export", id])
        .output()
        .context("failed to run `opencode` — is the OpenCode CLI in your PATH?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if stderr.is_empty() {
            bail!("`opencode session export {id}` failed ({})", output.status);
        }
        bail!(
            "`opencode session export {id}` failed ({}):\n{stderr}",
            output.status
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn safe_name(id: &str) -> String {
    let cleaned: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    if cleaned.trim_matches('-').is_empty() {
        "session".to_string()
    } else {
        cleaned
    }
}

fn open_in_browser(path: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("open");
        c.arg(path);
        c
    };

    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]).arg(path);
        c
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(path);
        c
    };

    let status = cmd.status().context("failed to launch browser")?;
    if !status.success() {
        bail!("browser opener exited with {status}");
    }
    Ok(())
}
