//! The `baukit` command line, as clap parses it.

use std::path::PathBuf;

use clap::{Args, CommandFactory, Parser, Subcommand};

use crate::{AuthProvider, Component, Components, QualityProfile, check::rule_names};

/// The full clap command tree, for help output and documentation tests.
#[must_use]
pub fn command() -> clap::Command {
    Cli::command()
}

fn rules_help() -> String {
    format!("Rules to run: {}. Default: all.", rule_names().join(", "))
}

#[derive(Debug, Parser)]
#[command(
    name = "baukit",
    version = crate::TEMPLATE_VERSION,
    about = "Generate and maintain Baukit products"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
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
pub struct CheckCommand {
    #[arg(help = rules_help())]
    pub rules: Vec<String>,
    /// Repository root to check.
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// Also print hint-level pattern matches.
    #[arg(long)]
    pub verbose: bool,
    /// agent-docs: write the managed CLAUDE.md block and the AGENTS.md symlink.
    #[arg(long)]
    pub fix: bool,
    /// suppressions: lower the maxima in baukit-check.toml to the current counts. template-files: rewrite the hashes.
    #[arg(long)]
    pub update: bool,
}

#[derive(Debug, Args)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "clap maps each on/off CLI flag to one bool field; NewCommand::components folds them into the typed Components set"
)]
pub struct NewCommand {
    /// Product name in lowercase kebab-case.
    pub name: String,
    /// Generate the Rust backend.
    #[arg(long)]
    pub backend: bool,
    /// Generate a durable PostgreSQL worker in the backend workspace.
    #[arg(long, requires = "backend")]
    pub worker: bool,
    /// Generate the Expo React Native application.
    #[arg(long)]
    pub mobile: bool,
    /// Generate the Vite React web application.
    #[arg(long)]
    pub web: bool,
    /// Build a PWA worker for the web app or Expo web export.
    #[arg(long)]
    pub pwa: bool,
    /// Generate an MCP server for the backend.
    #[arg(long, requires = "backend")]
    pub mcp: bool,
    /// Wire the platform receiver and entitlement cache into the backend.
    #[arg(long, requires = "backend")]
    pub platform: bool,
    /// Add an authentication capability.
    #[arg(long, value_enum)]
    pub auth: Option<AuthProvider>,
    /// Parent directory in which the product directory is created.
    #[arg(long, default_value = ".")]
    pub dir: PathBuf,
    /// Add missing files in a non-empty destination; modified files become conflicts.
    #[arg(long)]
    pub force: bool,
    /// Render directly into --dir, including an existing repository root.
    #[arg(long)]
    pub into_existing: bool,
    /// Skip network-backed Cargo/pnpm lockfile resolution.
    #[arg(long)]
    pub skip_lockfiles: bool,
    /// Use Baukit crates and packages from a local checkout's rust/ workspace.
    #[arg(long)]
    pub baukit_path: Option<PathBuf>,
    /// Add N to generated local service ports.
    #[arg(long, default_value_t = 0)]
    pub port_offset: u32,
    /// Select the generated CI and local quality-gate profile.
    #[arg(long, value_enum, default_value_t = QualityProfile::Standard)]
    pub quality: QualityProfile,
}

impl NewCommand {
    #[must_use]
    pub fn components(&self) -> Components {
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
pub enum GenerateCommand {
    /// Render the backend Dockerfile from baukit.toml.
    Dockerfile {
        /// Fail if the committed file differs from the render.
        #[arg(long)]
        check: bool,
    },
    /// Generate TypeScript declarations from the committed OpenAPI schema.
    OpenapiClient,
    /// Add the deploy/gitops overlays and the release and promote workflows that are missing.
    Gitops,
    /// Print the minimumReleaseAgeExclude entries a fresh @baukit/* re-pin needs, transitive packages included.
    ReleaseAgeExcludes,
}
