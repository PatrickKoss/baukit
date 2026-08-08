use std::{env, path::PathBuf, process::ExitCode};

use anyhow::Result;
use baukit_cli::{NewOptions, doctor, generate_new, generate_openapi_client};
use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "baukit",
    version,
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
    /// Generate the Expo React Native application.
    #[arg(long)]
    mobile: bool,
    /// Generate the Vite React web application.
    #[arg(long)]
    web: bool,
    /// Parent directory in which the product directory is created.
    #[arg(long, default_value = ".")]
    dir: PathBuf,
    /// Add missing files in a non-empty destination; modified files become conflicts.
    #[arg(long)]
    force: bool,
    /// Use Baukit crates and packages from a local checkout's rust/ workspace.
    #[arg(long)]
    baukit_path: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum GenerateCommand {
    /// Export OpenAPI and generate TypeScript declarations with openapi-typescript.
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
                mobile: command.mobile,
                web: command.web,
                force: command.force,
                baukit_path: command.baukit_path,
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
            println!("generated OpenAPI schema and TypeScript declarations");
        }
    }
    Ok(())
}
