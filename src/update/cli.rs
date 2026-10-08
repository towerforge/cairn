//! `cairn update`: the same update as `/update`, from a terminal and with
//! its options.

use std::io::{IsTerminal, Write};

use super::{Client, Layout, Target, Version, cache, target::APP};
use crate::store::Paths;

const USAGE: &str = "\
usage: cairn update [options]

Replaces this Cairn with the latest release from GitHub
(https://github.com/towerforge/cairn/releases).

  --check          only say whether there is a newer release
  --to <x.y.z>     install that version instead, downgrades included
  --force          reinstall the same version, or overwrite a cargo or
                   local build anyway
  -y, --yes        do not ask for confirmation
  -h, --help       this help";

struct Options {
    check: bool,
    to: Option<Version>,
    force: bool,
    yes: bool,
}

/// Runs `cairn update <args>` and returns the exit code.
pub fn run(args: &[String]) -> i32 {
    let opts = match parse(args) {
        Ok(Some(o)) => o,
        Ok(None) => {
            println!("{USAGE}");
            return 0;
        }
        Err(e) => {
            eprintln!("cairn update: {e}\n\n{USAGE}");
            return 2;
        }
    };
    match update(&opts) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("\n  ✗ {e:#}\n");
            1
        }
    }
}

fn parse(args: &[String]) -> anyhow::Result<Option<Options>> {
    let mut o = Options {
        check: false,
        to: None,
        force: false,
        yes: false,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => return Ok(None),
            "--check" => o.check = true,
            "--force" => o.force = true,
            "-y" | "--yes" => o.yes = true,
            "--to" => {
                let v = it
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--to needs a version"))?;
                o.to = Some(v.parse()?);
            }
            other => match other.strip_prefix("--to=") {
                Some(v) => o.to = Some(v.parse()?),
                None => anyhow::bail!("unknown option '{other}'"),
            },
        }
    }
    Ok(Some(o))
}

fn update(o: &Options) -> anyhow::Result<()> {
    let current = Version::current();
    let target = Target::current()?;
    let layout = Layout::current()?;
    let client = Client::new()?;

    println!();
    row("this", &format!("v{current} · {target}"));
    let release = match &o.to {
        Some(v) => client.release(v)?,
        None => client.latest()?,
    };
    let date = release
        .published_at
        .map(|d| format!(" · {}", d.format("%Y-%m-%d")))
        .unwrap_or_default();
    let label = if o.to.is_some() { "wanted" } else { "latest" };
    row(label, &format!("v{}{date}", release.version));

    let newer = release.version > current;
    if o.check {
        if newer {
            println!("\n  A new release is out: run `{APP} update`.\n");
        } else {
            println!("\n  ✓ Up to date.\n");
        }
        return Ok(());
    }
    // Without --to only forwards; with it, any other version.
    let wanted = match &o.to {
        Some(_) => release.version != current,
        None => newer,
    };
    if !wanted && !o.force {
        println!("\n  ✓ Up to date. `--force` reinstalls it.\n");
        return Ok(());
    }
    if let Some(why) = layout.kind().refusal()
        && !o.force
    {
        anyhow::bail!("{why} (or pass --force to overwrite it anyway)");
    }
    layout.check_writable()?;
    row("install to", &layout.path().display().to_string());

    let notes = release.highlights(6);
    if !notes.is_empty() {
        println!("\n  What's new");
        for n in notes {
            println!("    · {n}");
        }
    }
    if !release.url.is_empty() {
        println!("\n  {}", release.url);
    }
    if !o.yes && std::io::stdin().is_terminal() && !confirm(&release.version)? {
        println!("\n  Nothing was touched.\n");
        return Ok(());
    }

    println!();
    let mut last = None;
    let verified = super::apply(&client, &release, &target, &layout, |done, total| {
        let pct = total.filter(|t| *t > 0).map(|t| done * 100 / t);
        if pct != last {
            last = pct;
            let mb = |b: u64| b as f64 / 1_048_576.0;
            match (pct, total) {
                (Some(p), Some(t)) => {
                    eprint!("\r  downloading  {p:>3}%  {:.1}/{:.1} MB", mb(done), mb(t))
                }
                _ => eprint!("\r  downloading  {:.1} MB", mb(done)),
            }
        }
    })?;
    eprintln!();
    if verified {
        println!("  ✓ sha-256 verified");
    } else {
        println!("  ! no checksum published for this archive: not verified");
    }
    cache::CheckCache::forget(&Paths::from_env().state_dir);
    println!(
        "  ✓ Cairn v{} installed at {}\n\n  Restart Cairn to use it.\n",
        release.version,
        layout.path().display()
    );
    Ok(())
}

fn row(key: &str, value: &str) {
    println!("  {key:<11} {value}");
}

fn confirm(v: &Version) -> anyhow::Result<bool> {
    print!("\n  Install v{v}? [Y/n] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "" | "y" | "Y" | "yes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn parses_the_options() {
        let o = parse(&args("--check --to 0.4.0 -y")).unwrap().unwrap();
        assert!(o.check && o.yes && !o.force);
        assert_eq!(o.to, Some("0.4.0".parse().unwrap()));
        let o = parse(&args("--to=v1.0.0 --force")).unwrap().unwrap();
        assert_eq!(o.to, Some("1.0.0".parse().unwrap()));
        assert!(parse(&args("--help")).unwrap().is_none());
        assert!(parse(&args("--to")).is_err());
        assert!(parse(&args("--to banana")).is_err());
        assert!(parse(&args("--wat")).is_err());
    }
}
