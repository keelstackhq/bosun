#![forbid(unsafe_code)]

use bosun::cli::COMMANDS;
use bosun::{BosunError, Cli, CmdResult, envelope};
use clap::Parser;
use clap::error::ErrorKind;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => return usage_error(&err),
    };
    let command = cli.command.name();
    let result = bosun::run(cli);
    print(command, &result);
    ExitCode::from(bosun::output::exit_of(&result).code())
}

fn print(command: &str, result: &CmdResult) {
    if let Err(e) = result {
        eprintln!("bosun {command}: {}", e.message);
    }
    println!("{}", envelope(command, result));
}

/// Help and version print clap's text (exit 0). Every other parse error is a usage error:
/// the clap text goes to stderr and one envelope (exit 2) to stdout.
fn usage_error(err: &clap::Error) -> ExitCode {
    if matches!(
        err.kind(),
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
    ) {
        let _ = err.print();
        return ExitCode::from(0);
    }
    let text = err.render().to_string();
    eprint!("{text}");
    let message = if err.kind() == ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand {
        "a subcommand is missing".to_string()
    } else {
        let first = text.lines().next().unwrap_or("usage error");
        first.strip_prefix("error: ").unwrap_or(first).to_string()
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = command_from_args(&args);
    let fix = if command == "unknown" {
        "run `bosun --help` for the commands".to_string()
    } else {
        format!("run `bosun {command} --help` for its arguments")
    };
    let result: CmdResult = Err(BosunError::usage(message, fix));
    println!("{}", envelope(command, &result));
    ExitCode::from(2)
}

/// The first positional argument if it names a command, else `"unknown"`.
fn command_from_args(args: &[String]) -> &'static str {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "-C" {
            iter.next();
        } else if !arg.starts_with('-') {
            return COMMANDS.into_iter().find(|c| c == arg).unwrap_or("unknown");
        }
    }
    "unknown"
}
