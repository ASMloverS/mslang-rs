use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "ms", about = "mslang scripting language")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(short, long)]
    version: bool,
}

#[derive(Subcommand)]
enum Commands {
    Run { file: String },
    Eval { expr: String },
    Repl,
    Check { file: String },
}

fn main() {
    let cli = Cli::parse();
    if cli.version {
        println!("mslang 0.1.0");
        return;
    }
    match cli.command {
        Some(Commands::Run { file: _ }) => {}
        Some(Commands::Eval { expr: _ }) => {}
        Some(Commands::Repl) => {}
        Some(Commands::Check { file: _ }) => {}
        None => { Cli::parse_from(["ms", "--help"]); }
    }
}
