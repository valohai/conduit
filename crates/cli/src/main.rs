use clap::{Parser, Subcommand};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "TODO: remove")]
    Greet,
    #[command(about = "TODO: remove")]
    Error,
    #[command(about = "TODO: remove")]
    Panic,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None => println!("Default command!"),
        Some(Commands::Greet) => println!("{}", conduit_proxy::greeting()),
        Some(Commands::Error) => anyhow::bail!("error from cli"),
        Some(Commands::Panic) => panic!("panic from cli"),
    }
    Ok(())
}
