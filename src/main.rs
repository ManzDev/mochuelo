mod icons;
mod model;
mod pricing;
mod render;
mod server;
mod sessions;
mod tui;

use anyhow::{bail, Context, Result};
use clap::Parser;
use std::fs;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use model::Export;

/// Inspect OpenCode sessions as a local web page.
#[derive(Debug, Parser)]
#[command(name = "mochuelo", version, about, long_about = None)]
struct Cli {
    /// Session id (ses_...), a path to an exported .json file, or `-` for stdin.
    /// Omit it to pick a session interactively.
    session: Option<String>,

    /// Write a standalone HTML file instead of starting the local server
    #[arg(short, long, value_name = "FILE", conflicts_with = "stdout")]
    output: Option<PathBuf>,

    /// Print the HTML to stdout instead of serving or writing it
    #[arg(long, conflicts_with = "no_open")]
    stdout: bool,

    /// Don't open the page in the default browser
    #[arg(long)]
    no_open: bool,

    /// Port for the local server (0 = random free port)
    #[arg(long, default_value_t = 0, value_name = "PORT")]
    port: u16,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(path) = &cli.output {
        let Some(source) = &cli.session else {
            bail!("`-o/--output` requires a session id, a .json export, or `-` for stdin");
        };
        let export = load_export(source)?;
        write_page(&render::render(&export), path)?;
        println!("{}", path.display());
        return Ok(());
    }

    if let Some(source) = &cli.session {
        let export = load_export(source)?;
        let html = render::render(&export);

        if cli.stdout {
            let mut out = std::io::stdout().lock();
            out.write_all(html.as_bytes())?;
            return Ok(());
        }

        return serve(html, cli.port, !cli.no_open);
    }

    if cli.stdout {
        bail!("`--stdout` requires a session id or `-o/--output`");
    }
    if !std::io::stdin().is_terminal() {
        bail!("the interactive picker needs a terminal; pass a session id instead");
    }

    tui::run(cli.port, !cli.no_open)
}

pub(crate) fn load_export(arg: &str) -> Result<Export> {
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

fn write_page(html: &str, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
    }
    fs::write(path, html).with_context(|| format!("failed to write {}", path.display()))
}

fn serve(html: String, port: u16, open: bool) -> Result<()> {
    let server = server::Server::start(html, port).context("failed to start local server")?;

    println!("Serving session at {}", server.url());
    println!("Press Ctrl+C to stop.");

    if open {
        if let Err(err) = server.wait_ready() {
            eprintln!("warning: server did not become ready: {err}");
        }
        if let Err(err) = open_url(server.url()) {
            eprintln!("warning: could not open browser: {err}");
        }
    }

    loop {
        std::thread::park();
    }
}

pub(crate) fn open_url(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("open");
        c.arg(url);
        c
    };

    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]).arg(url);
        c
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };

    cmd.spawn().context("failed to launch browser")?;
    Ok(())
}
