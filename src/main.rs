//! Cairn — block-based terminal for macOS and Linux.
//!
//! It opens its own window (`gui`); all the logic lives in `App`.
//!
//! Each command is a block with its output, exit code and duration.
//! Full-screen programs (vim, htop, less…) take over the whole view while
//! they run.

#[cfg(not(unix))]
compile_error!("Cairn only runs on macOS and Linux");

mod app;
mod edit;
mod event;
mod git;
mod gui;
mod store;
mod term;
mod update;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "update") {
        std::process::exit(update::cli::run(&args[1..]));
    }
    for arg in &args {
        match arg.as_str() {
            "-V" | "--version" => {
                println!("cairn {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "-h" | "--help" => {
                println!(
                    "cairn {} — block-based terminal\n",
                    env!("CARGO_PKG_VERSION")
                );
                println!("usage: cairn\n       cairn update [--check] [--to x.y.z]\n");
                println!("  (no options)    opens the Cairn window");
                println!("  update          installs the latest release (--help for more)\n");
                println!("Inside Cairn, Ctrl+G shows the shortcuts menu.");
                println!("`edit <file>` opens the side editor.\n");
                let paths = store::Paths::from_env();
                println!("Files (XDG_CONFIG_HOME and XDG_DATA_HOME are honoured):");
                println!("  profiles & snippets  {}", paths.config_file().display());
                println!("  history              {}", paths.history_file().display());
                println!("  update check         {}", paths.state_dir.display());
                return;
            }
            // macOS adds -psn_… when old-style apps are opened from the Finder.
            a if a.starts_with("-psn_") => {}
            other => {
                eprintln!("cairn: unknown argument '{other}' (try --help)");
                std::process::exit(2);
            }
        }
    }
    let result = store::Store::open().and_then(gui::run);
    if let Err(e) = result {
        eprintln!("cairn: {e:#}");
        std::process::exit(1);
    }
}
