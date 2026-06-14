use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "ms", about = "mslang scripting language")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(long)]
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
        println!("mslang {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    match cli.command {
        Some(Commands::Run { file: _ }) => {}
        Some(Commands::Eval { expr: _ }) => {}
        Some(Commands::Repl) => {}
        Some(Commands::Check { file: _ }) => {}
        None => {
            use clap::CommandFactory;
            Cli::command().print_help().ok();
        }
    }
}
