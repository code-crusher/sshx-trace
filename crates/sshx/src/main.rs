use std::process::ExitCode;

use ansi_term::Color::{Cyan, Fixed, Green};
use anyhow::{Context, Result};
use clap::Parser;
use sshx::{
    controller::{Controller, ControllerOptions},
    runner::Runner,
    terminal::get_default_shell,
};
use tokio::signal;
use tracing::error;

/// A secure web-based, collaborative terminal.
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Address of the remote sshx server.
    #[clap(long, default_value = "https://sshx.io", env = "SSHX_SERVER")]
    server: String,

    /// Local shell command to run in the terminal.
    #[clap(long)]
    shell: Option<String>,

    /// Quiet mode, only prints the URL to stdout.
    #[clap(short, long)]
    quiet: bool,

    /// Session name displayed in the title (defaults to user@hostname).
    #[clap(long)]
    name: Option<String>,

    /// Enable read-only access mode - generates separate URLs for viewers and
    /// editors.
    #[clap(long)]
    enable_readers: bool,

    /// File or directory with PEM-encoded CA certificates to trust, in addition
    /// to the built-in root certificates.
    #[clap(long, value_name = "PATH", env = "SSHX_CACERT")]
    cacert: Option<String>,
}

/// Read PEM-encoded CA certificates from a file or from every file in a
/// directory, concatenating them into a single bundle.
fn read_ca_certificates(path: &str) -> Result<Vec<u8>> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("failed to read CA certificates: {path}"))?;
    if !metadata.is_dir() {
        return std::fs::read(path)
            .with_context(|| format!("failed to read CA certificates: {path}"));
    }

    let mut bundle = Vec::new();
    let entries =
        std::fs::read_dir(path).with_context(|| format!("failed to read CA directory: {path}"))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("failed to read CA directory: {path}"))?;
        let file_path = entry.path();
        if file_path.is_file() {
            bundle.extend_from_slice(&std::fs::read(&file_path).with_context(|| {
                format!("failed to read CA certificate: {}", file_path.display())
            })?);
            bundle.push(b'\n');
        }
    }
    Ok(bundle)
}

fn print_greeting(shell: &str, controller: &Controller) {
    let version_str = match option_env!("CARGO_PKG_VERSION") {
        Some(version) => format!("v{version}"),
        None => String::from("[dev]"),
    };
    if let Some(write_url) = controller.write_url() {
        println!(
            r#"
  {sshx} {version}

  {arr}  Read-only link: {link_v}
  {arr}  Writable link:  {link_e}
  {arr}  Shell:          {shell_v}
"#,
            sshx = Green.bold().paint("sshx"),
            version = Green.paint(&version_str),
            arr = Green.paint("➜"),
            link_v = Cyan.underline().paint(controller.url()),
            link_e = Cyan.underline().paint(write_url),
            shell_v = Fixed(8).paint(shell),
        );
    } else {
        println!(
            r#"
  {sshx} {version}

  {arr}  Link:  {link_v}
  {arr}  Shell: {shell_v}
"#,
            sshx = Green.bold().paint("sshx"),
            version = Green.paint(&version_str),
            arr = Green.paint("➜"),
            link_v = Cyan.underline().paint(controller.url()),
            shell_v = Fixed(8).paint(shell),
        );
    }
}

#[tokio::main]
async fn start(args: Args) -> Result<()> {
    let shell = match args.shell {
        Some(shell) => shell,
        None => get_default_shell().await,
    };

    let name = args.name.unwrap_or_else(|| {
        let mut name = whoami::username();
        if let Ok(host) = whoami::fallible::hostname() {
            // Trim domain information like .lan or .local
            let host = host.split('.').next().unwrap_or(&host);
            name += "@";
            name += host;
        }
        name
    });

    let ca_certificates = args
        .cacert
        .as_deref()
        .map(read_ca_certificates)
        .transpose()?;

    let runner = Runner::Shell(shell.clone());
    let mut options = ControllerOptions::default();
    options.enable_readers = args.enable_readers;
    options.ca_certificates = ca_certificates;
    let mut controller = Controller::new_with_options(&args.server, &name, runner, options).await?;
    if args.quiet {
        if let Some(write_url) = controller.write_url() {
            println!("{}", write_url);
        } else {
            println!("{}", controller.url());
        }
    } else {
        print_greeting(&shell, &controller);
    }

    let exit_signal = signal::ctrl_c();
    tokio::pin!(exit_signal);
    tokio::select! {
        _ = controller.run() => unreachable!(),
        Ok(()) = &mut exit_signal => (),
    };
    controller.close().await?;

    Ok(())
}

fn main() -> ExitCode {
    let args = Args::parse();

    let default_level = if args.quiet { "error" } else { "info" };

    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or(default_level.into()))
        .with_writer(std::io::stderr)
        .init();

    match start(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            error!("{err:?}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_ca_certificates_from_file() -> Result<()> {
        let path = format!("{}/testdata/ca.pem", env!("CARGO_MANIFEST_DIR"));
        let bundle = read_ca_certificates(&path)?;
        assert!(bundle.starts_with(b"-----BEGIN CERTIFICATE-----"));
        assert!(bundle.ends_with(b"-----END CERTIFICATE-----\n"));
        Ok(())
    }

    #[test]
    fn read_ca_certificates_from_directory() -> Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "sshx-cacert-test-{}",
            sshx_core::rand_alphanumeric(10)
        ));
        std::fs::create_dir(&dir)?;
        let ca = std::fs::read(format!("{}/testdata/ca.pem", env!("CARGO_MANIFEST_DIR")))?;
        let server = std::fs::read(format!(
            "{}/testdata/server.pem",
            env!("CARGO_MANIFEST_DIR")
        ))?;
        std::fs::write(dir.join("ca.pem"), &ca)?;
        std::fs::write(dir.join("server.pem"), &server)?;
        // Subdirectories and other entries are ignored.
        std::fs::create_dir(dir.join("nested"))?;

        let bundle = read_ca_certificates(dir.to_str().unwrap())?;
        std::fs::remove_dir_all(&dir)?;

        assert_eq!(bundle.len(), ca.len() + 1 + server.len() + 1);
        Ok(())
    }

    #[test]
    fn read_ca_certificates_missing_path() {
        assert!(read_ca_certificates("/nonexistent/sshx/ca.pem").is_err());
    }
}
