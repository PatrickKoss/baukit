use std::{env, path::PathBuf, process::ExitCode};

use anyhow::Result;
use baukit_cli::{
    AuthProvider, Component, Components, NewOptions, QualityProfile,
    check::{CheckOptions, CheckRequest, run_check},
    doctor, generate_dockerfile, generate_new, generate_openapi_client,
};
use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "baukit",
    version = baukit_cli::TEMPLATE_VERSION,
    about = "Generate and maintain Baukit products"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Generate a new product.
    New(NewCommand),
    /// Validate the generated product in the current directory.
    Doctor,
    /// Generate derived product artifacts.
    Generate {
        #[command(subcommand)]
        command: GenerateCommand,
    },
    /// Enforce the shared baukit rules; exits 1 on findings, 2 on usage or config errors.
    Check(CheckCommand),
}

#[derive(Debug, Args)]
struct CheckCommand {
    /// Rules to run: lints, file-size, suppressions, patterns, agent-docs. Default: all.
    rules: Vec<String>,
    /// Repository root to check.
    #[arg(long, default_value = ".")]
    root: PathBuf,
    /// Also print hint-level pattern matches.
    #[arg(long)]
    verbose: bool,
    /// agent-docs: write the managed CLAUDE.md block and the AGENTS.md symlink.
    #[arg(long)]
    fix: bool,
    /// suppressions: lower the maxima in baukit-check.toml to the current counts.
    #[arg(long)]
    update: bool,
}

const CHECK_FINDINGS_EXIT: u8 = 1;
const CHECK_USAGE_EXIT: u8 = 2;

#[derive(Debug, Args)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "clap maps each on/off CLI flag to one bool field; NewCommand::components folds them into the typed Components set"
)]
struct NewCommand {
    /// Product name in lowercase kebab-case.
    name: String,
    /// Generate the Rust backend.
    #[arg(long)]
    backend: bool,
    /// Generate a durable PostgreSQL worker in the backend workspace.
    #[arg(long, requires = "backend")]
    worker: bool,
    /// Generate the Expo React Native application.
    #[arg(long)]
    mobile: bool,
    /// Generate the Vite React web application.
    #[arg(long)]
    web: bool,
    /// Build a PWA worker for the web app or Expo web export.
    #[arg(long)]
    pwa: bool,
    /// Generate an MCP server for the backend.
    #[arg(long, requires = "backend")]
    mcp: bool,
    /// Wire the platform receiver and entitlement cache into the backend.
    #[arg(long, requires = "backend")]
    platform: bool,
    /// Add an authentication capability.
    #[arg(long, value_enum)]
    auth: Option<AuthProvider>,
    /// Parent directory in which the product directory is created.
    #[arg(long, default_value = ".")]
    dir: PathBuf,
    /// Add missing files in a non-empty destination; modified files become conflicts.
    #[arg(long)]
    force: bool,
    /// Render directly into --dir, including an existing repository root.
    #[arg(long)]
    into_existing: bool,
    /// Skip network-backed Cargo/pnpm lockfile resolution.
    #[arg(long)]
    skip_lockfiles: bool,
    /// Use Baukit crates and packages from a local checkout's rust/ workspace.
    #[arg(long)]
    baukit_path: Option<PathBuf>,
    /// Add N to generated local service ports.
    #[arg(long, default_value_t = 0)]
    port_offset: u32,
    /// Select the generated CI and local quality-gate profile.
    #[arg(long, value_enum, default_value_t = QualityProfile::Standard)]
    quality: QualityProfile,
}

impl NewCommand {
    fn components(&self) -> Components {
        [
            (Component::Backend, self.backend),
            (Component::Worker, self.worker),
            (Component::Mobile, self.mobile),
            (Component::Web, self.web),
            (Component::Pwa, self.pwa),
            (Component::Mcp, self.mcp),
            (Component::Platform, self.platform),
        ]
        .into_iter()
        .filter_map(|(component, enabled)| enabled.then_some(component))
        .collect()
    }
}

#[derive(Debug, Subcommand)]
enum GenerateCommand {
    /// Render the backend Dockerfile from baukit.toml.
    Dockerfile {
        /// Fail if the committed file differs from the render.
        #[arg(long)]
        check: bool,
    },
    /// Generate TypeScript declarations from the committed OpenAPI schema.
    OpenapiClient,
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Commands::Check(command) => check(&command),
        Commands::New(command) => report(new_product(command)),
        Commands::Doctor => report(doctor_report()),
        Commands::Generate { command } => report(generate(&command)),
    }
}

#[expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "CLI user output: `baukit check` prints findings and maps them to its exit code"
)]
fn check(command: &CheckCommand) -> ExitCode {
    let request = CheckRequest {
        root: command.root.clone(),
        rules: command.rules.clone(),
        options: CheckOptions {
            verbose: command.verbose,
            fix: command.fix,
            update: command.update,
        },
    };
    match run_check(&request) {
        Ok(report) => {
            for line in &report.lines {
                println!("{line}");
            }
            if report.failed {
                ExitCode::from(CHECK_FINDINGS_EXIT)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(CHECK_USAGE_EXIT)
        }
    }
}

#[expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "CLI user output: the single place the binary writes to the terminal"
)]
fn report(outcome: Result<Vec<String>>) -> ExitCode {
    match outcome {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn generate(command: &GenerateCommand) -> Result<Vec<String>> {
    match *command {
        GenerateCommand::Dockerfile { check } => {
            let path = generate_dockerfile(&env::current_dir()?, check)?;
            let verb = if check { "checked" } else { "generated" };
            Ok(vec![format!("{verb} {}", path.display())])
        }
        GenerateCommand::OpenapiClient => {
            generate_openapi_client(&env::current_dir()?)?;
            Ok(vec![
                "generated TypeScript declarations from the committed OpenAPI schema".to_owned(),
            ])
        }
    }
}

fn new_product(command: NewCommand) -> Result<Vec<String>> {
    let components = command.components();
    let destination = generate_new(&NewOptions {
        name: command.name,
        directory: command.dir,
        components,
        auth: command.auth,
        force: command.force,
        into_existing: command.into_existing,
        resolve_lockfiles: !command.skip_lockfiles,
        baukit_path: command.baukit_path,
        port_offset: command.port_offset,
        quality: command.quality,
    })?;
    Ok(vec![format!("generated {}", destination.display())])
}

fn doctor_report() -> Result<Vec<String>> {
    let mut lines = doctor(&env::current_dir()?)?
        .into_iter()
        .map(|result| {
            if result.starts_with("note: ") {
                result
            } else {
                format!("ok: {result}")
            }
        })
        .collect::<Vec<_>>();
    lines.push("doctor: product is healthy".to_owned());
    Ok(lines)
}
