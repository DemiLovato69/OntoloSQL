use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Generate OSDK ontology definitions from PostgreSQL schema files"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Generate {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    ExtractCopy {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(short = 'd', long)]
        output_dir: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Generate { input, output } => {
            ontolosql::generate_file(ontolosql::GenerateOptions { input, output })?;
        }
        Command::ExtractCopy { input, output_dir } => {
            ontolosql::extract_copy_to_csv_files(ontolosql::ExtractCopyOptions {
                input,
                output_dir,
            })?;
        }
    }

    Ok(())
}
