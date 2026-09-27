//! supertools — agent-native harnesses for existing developer tools.
//!
//! v0.1 domains: search, repo, verify — plus discovery/teaching
//! (doctor, describe, capabilities, teach, tools, find).
//!
//! One canonical output envelope, one operation catalogue, one tool
//! registry. Machine-first, bounded, read-only by default, fail closed.

mod capabilities;
mod catalogue;
mod cli;
mod describe;
mod doctor;
mod output;
mod present;
mod process;
mod registry;
mod repo;
mod search;
mod teach;
mod tools_cmd;
mod verify;

use clap::Parser;
use cli::{Cli, Command, RepoCommand, SearchCommand, ToolsCommand, VerifyCommand};
use search::CommonOpts;

fn dispatch(cmd: Command) -> output::CmdResult {
    match cmd {
        Command::Doctor => doctor::doctor_cmd(),
        Command::Capabilities => capabilities::capabilities_cmd(),
        Command::Describe { target } => describe::describe_cmd(target),
        Command::Teach {
            topic,
            target,
            path,
            apply,
        } => teach::teach_cmd(&topic, target, path, apply),
        Command::Tools { cmd } => match cmd {
            None => tools_cmd::tools_list(),
            Some(ToolsCommand::Show { tool }) => tools_cmd::tools_show(tool),
            Some(ToolsCommand::Missing) => tools_cmd::tools_filter("missing"),
            Some(ToolsCommand::Hidden) => tools_cmd::tools_filter("hidden"),
            Some(ToolsCommand::Available) => tools_cmd::tools_filter("available"),
            Some(ToolsCommand::InstallMissing { yes }) => tools_cmd::tools_install_missing(yes),
            Some(ToolsCommand::FixPath { dry_run, yes }) => tools_cmd::tools_fix_path(dry_run, yes),
        },
        Command::Find {
            tool,
            install,
            install_missing,
            fix_path,
            dry_run,
            yes,
        } => tools_cmd::find(tool, install, install_missing, fix_path, dry_run, yes),
        Command::Search { cmd } => match cmd {
            SearchCommand::Text {
                query,
                limit,
                path,
                fixed,
                globs,
                ..
            } => search::text(CommonOpts {
                query,
                limit,
                path,
                fixed,
                globs,
                context: 0,
            }),
            SearchCommand::Files {
                query,
                limit,
                path,
                fixed,
            } => search::files(CommonOpts {
                query,
                limit,
                path,
                fixed,
                globs: Vec::new(),
                context: 0,
            }),
            SearchCommand::Context {
                query,
                limit,
                path,
                fixed,
                context,
                ..
            } => search::context(CommonOpts {
                query,
                limit,
                path,
                fixed,
                globs: Vec::new(),
                context,
            }),
            SearchCommand::Symbol { query, limit, path } => search::symbol(CommonOpts {
                query,
                limit,
                path,
                fixed: false,
                globs: Vec::new(),
                context: 0,
            }),
            SearchCommand::Structural {
                pattern,
                limit,
                path,
            } => search::structural(CommonOpts {
                query: pattern,
                limit,
                path,
                fixed: true,
                globs: Vec::new(),
                context: 0,
            }),
        },
        Command::Repo { cmd } => match cmd {
            RepoCommand::State => repo::state(),
            RepoCommand::Changed { limit } => repo::changed(limit),
            RepoCommand::History { target, limit } => repo::history(target, limit),
            RepoCommand::Remote => repo::remote(),
            RepoCommand::Pr => repo::pr(),
            RepoCommand::Ci { limit } => repo::ci(limit),
        },
        Command::Verify { cmd } => match cmd {
            VerifyCommand::Discover => verify::discover_cmd(),
            VerifyCommand::Quick { timeout } => verify::quick(timeout),
            VerifyCommand::Full { timeout } => verify::full(timeout),
            VerifyCommand::Task { name, timeout } => verify::task(name, timeout),
        },
    }
}

fn render(b: output::Builder, json: bool) -> i32 {
    let code = b.status.exit_code();
    if json {
        match serde_json::to_string_pretty(&b.envelope()) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("supertools: could not serialise envelope: {e}"),
        }
    } else {
        // Sartorial dresses the human body for attached terminals only;
        // piped output keeps the deterministic plain lines byte-for-byte.
        if !present::render_builder(&b) {
            if b.human.is_empty() {
                println!("{}", b.summary);
            } else {
                for l in &b.human {
                    println!("{l}");
                }
            }
        }
        // Semantic facts stay on stderr in human mode, styled or not.
        for w in &b.warnings {
            eprintln!("warning: {w}");
        }
        for n in &b.next_actions {
            if n.command.is_empty() {
                eprintln!("hint: {}", n.reason);
            } else {
                eprintln!("next: {}  — {}", n.command, n.reason);
            }
        }
    }
    code
}

fn render_failure(f: output::Failure, json: bool) -> i32 {
    let code = f.status.exit_code();
    if json {
        match serde_json::to_string_pretty(&f.envelope()) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("supertools: could not serialise envelope: {e}"),
        }
    } else if !present::render_failure(&f) {
        eprintln!("supertools error [{}]: {}", f.status.as_str(), f.message);
        for h in &f.hints {
            if h.is_empty() {
                continue;
            }
            eprintln!("  hint: {h}");
        }
    }
    code
}

fn main() {
    let cli = Cli::parse();
    present::set_json_mode(cli.json);
    let code = match dispatch(cli.cmd) {
        Ok(b) => render(b, cli.json),
        Err(f) => render_failure(f, cli.json),
    };
    std::process::exit(code);
}
