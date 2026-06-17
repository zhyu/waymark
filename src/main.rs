use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};

use waymark::db::{Database, Kind};
use waymark::{Result, err};

#[derive(Debug, Parser)]
#[command(name = "waymark")]
#[command(about = "A zsh-first fasd replacement")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Add(AddArgs),
    Query(QueryArgs),
    Import(ImportCommand),
    Init(InitCommand),
    Doctor,
    Delete(DeleteArgs),
    Prune(PruneArgs),
    Dump(DumpArgs),
}

#[derive(Debug, Args)]
struct AddArgs {
    #[arg(long, value_enum, default_value_t = AddKind::Auto)]
    kind: AddKind,
    #[arg(required = true, trailing_var_arg = true)]
    paths: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum AddKind {
    File,
    Dir,
    Auto,
}

#[derive(Debug, Args)]
struct QueryArgs {
    #[arg(long, value_enum, default_value_t = QueryKind::Any)]
    kind: QueryKind,
    #[arg(long)]
    best: bool,
    #[arg(long)]
    score: bool,
    #[arg(long, default_value_t = 20)]
    limit: usize,
    #[arg(long)]
    interactive: bool,
    #[arg(long, value_enum, default_value_t = OutputFormat::Plain)]
    format: OutputFormat,
    #[arg(trailing_var_arg = true)]
    query: Vec<String>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum QueryKind {
    File,
    Dir,
    Any,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum OutputFormat {
    Plain,
    Json,
}

#[derive(Debug, Args)]
struct ImportCommand {
    #[command(subcommand)]
    command: ImportSubcommand,
}

#[derive(Debug, Subcommand)]
enum ImportSubcommand {
    Fasd(FasdArgs),
}

#[derive(Debug, Args)]
struct FasdArgs {
    #[arg(long = "from")]
    from: Option<PathBuf>,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    keep_missing: bool,
    #[arg(long)]
    strict: bool,
}

#[derive(Debug, Args)]
struct InitCommand {
    #[command(subcommand)]
    command: InitSubcommand,
}

#[derive(Debug, Subcommand)]
enum InitSubcommand {
    Zsh,
}

#[derive(Debug, Args)]
struct DeleteArgs {
    #[arg(required = true, trailing_var_arg = true)]
    paths: Vec<PathBuf>,
}

#[derive(Debug, Args)]
struct PruneArgs {
    #[arg(long)]
    missing: bool,
}

#[derive(Debug, Args)]
struct DumpArgs {
    #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
    format: OutputFormat,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("waymark: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Add(args) => {
            let db = Database::open_default()?;
            let kind = match args.kind {
                AddKind::File => Kind::File,
                AddKind::Dir => Kind::Dir,
                AddKind::Auto => Kind::Auto,
            };
            db.add_paths(kind, &args.paths)?;
        }
        Command::Query(args) => {
            let db = Database::open_default()?;
            let kind = match args.kind {
                QueryKind::File => Kind::File,
                QueryKind::Dir => Kind::Dir,
                QueryKind::Any => Kind::Any,
            };
            let results = db.query(kind, &args.query, args.limit.max(1))?;
            if args.format == OutputFormat::Json {
                if args.interactive {
                    let selected = waymark::ranking::select_interactive(&results)?;
                    serde_json::to_writer_pretty(std::io::stdout(), &selected)?;
                } else if args.best {
                    serde_json::to_writer_pretty(std::io::stdout(), &results.first())?;
                } else {
                    serde_json::to_writer_pretty(std::io::stdout(), &results)?;
                }
                println!();
            } else if args.interactive {
                match waymark::ranking::select_interactive(&results)? {
                    Some(path) => println!("{path}"),
                    None => return Err(err("no matching path")),
                }
            } else if args.best {
                let Some(result) = results.first() else {
                    return Err(err("no matching path"));
                };
                if args.score {
                    println!("{:.6}\t{}", result.score, result.entry.path);
                } else {
                    println!("{}", result.entry.path);
                }
            } else {
                for result in results {
                    if args.score {
                        println!("{:.6}\t{}", result.score, result.entry.path);
                    } else {
                        println!("{}", result.entry.path);
                    }
                }
            }
        }
        Command::Import(args) => match args.command {
            ImportSubcommand::Fasd(args) => {
                let options = waymark::fasd::FasdImportOptions {
                    dry_run: args.dry_run,
                    keep_missing: args.keep_missing,
                    strict: args.strict,
                };
                let summary = waymark::fasd::import_fasd(args.from.as_deref(), options)?;
                println!(
                    "parsed={} imported={} skipped={} malformed={} missing={} files={} dirs={} unknown={}",
                    summary.parsed,
                    summary.imported,
                    summary.skipped,
                    summary.malformed,
                    summary.missing,
                    summary.files,
                    summary.dirs,
                    summary.unknown
                );
            }
        },
        Command::Init(args) => match args.command {
            InitSubcommand::Zsh => print!("{}", waymark::zsh::init_script()),
        },
        Command::Doctor => {
            let report = Database::doctor()?;
            print!("{report}");
        }
        Command::Delete(args) => {
            let db = Database::open_default()?;
            db.delete_paths(&args.paths)?;
        }
        Command::Prune(args) => {
            if !args.missing {
                return Err(err("only prune --missing is supported"));
            }
            let db = Database::open_default()?;
            db.prune_missing()?;
        }
        Command::Dump(args) => {
            let db = Database::open_default()?;
            match args.format {
                OutputFormat::Json => {
                    serde_json::to_writer_pretty(std::io::stdout(), &db.all_entries()?)?;
                    println!();
                }
                OutputFormat::Plain => {
                    for entry in db.all_entries()? {
                        println!("{}", entry.path);
                    }
                }
            }
        }
    }
    Ok(())
}
