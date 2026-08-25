use std::{env, path::PathBuf, process::ExitCode};

use anyhow::Result;
use baukit_cli::{AuthProvider, NewOptions, doctor, generate_new, generate_openapi_client};
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
}

#[derive(Debug, Args)]
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
}

#[derive(Debug, Subcommand)]
enum GenerateCommand {
    /// Generate TypeScript declarations from the committed OpenAPI schema.
    OpenapiClient,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Commands::New(command) => {
            let destination = generate_new(&NewOptions {
                name: command.name,
                directory: command.dir,
                backend: command.backend,
                worker: command.worker,
                mobile: command.mobile,
                web: command.web,
                auth: command.auth,
                force: command.force,
                into_existing: command.into_existing,
                resolve_lockfiles: !command.skip_lockfiles,
                baukit_path: command.baukit_path,
                port_offset: command.port_offset,
            })?;
            println!("generated {}", destination.display());
        }
        Commands::Doctor => {
            let current = env::current_dir()?;
            for result in doctor(&current)? {
                println!("ok: {result}");
            }
            println!("doctor: product is healthy");
        }
        Commands::Generate {
            command: GenerateCommand::OpenapiClient,
        } => {
            generate_openapi_client(&env::current_dir()?)?;
            println!("generated TypeScript declarations from the committed OpenAPI schema");
        }
    }
    Ok(())
}
