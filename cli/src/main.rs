use std::{env, process::ExitCode};

use anyhow::Result;
use baukit_cli::{
    NewOptions,
    check::{CheckOptions, CheckRequest, run_check},
    cli::{CheckCommand, Cli, Commands, GenerateCommand, NewCommand},
    doctor, generate_dockerfile, generate_gitops, generate_new, generate_openapi_client,
    release_age_excludes,
};
use clap::Parser;

const CHECK_FINDINGS_EXIT: u8 = 1;
const CHECK_USAGE_EXIT: u8 = 2;

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
        GenerateCommand::Gitops => generate_gitops(&env::current_dir()?),
        GenerateCommand::ReleaseAgeExcludes => release_age_excludes(&env::current_dir()?),
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
