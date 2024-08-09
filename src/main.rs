use std::collections::BTreeMap;
use std::error::Error;
use std::io::{BufRead, Write};
use std::process::{Command, ExitCode, Stdio};
use std::{collections::BTreeSet, path::PathBuf};

use clap::Parser;

#[derive(Debug, clap::Parser)]
struct Args {
    /// Don't rename any files, show what would be renamed
    #[arg(long, short = 'n')]
    dry_run: bool,

    /// The editor to use
    #[arg(long, short, env = "EDITOR")]
    editor: PathBuf,

    /// The files to rename, pass `-` to read form stdin
    #[arg(required = true, num_args(1..))]
    files: Vec<String>,
}

fn main() -> ExitCode {
    main_impl().unwrap()
}

fn get_ancestor(path: &str) -> Option<&str> {
    path.strip_suffix('/')
        .unwrap_or(path)
        .rsplit_once('/')
        .map(|(stem, _)| stem)
}

fn main_impl() -> Result<ExitCode, Box<dyn Error>> {
    let mut args = Args::parse();

    if args.files.len() == 1 && args.files.first().is_some_and(|f| f == "-") {
        args.files.clear();

        let stdin = std::io::stdin().lock();
        for line in stdin.lines() {
            let line = line?;
            args.files.push(line);
        }

        if args.files.is_empty() {
            eprintln!("no files provided on stdin");
            return Ok(ExitCode::FAILURE);
        }
    }

    // order and deduplicate
    let before: BTreeSet<_> = args.files.iter().cloned().collect();
    let before: Vec<_> = before.into_iter().collect();

    let mut buffer = vec![];

    // write header and paths
    writeln!(buffer, "// empty lines and coments are ignored")?;
    writeln!(buffer, "// do not remove or reorder any lines")?;
    writeln!(buffer, "// do not edit anything other than file stems")?;
    writeln!(buffer)?;
    for file in &before {
        buffer.write_all(file.as_bytes())?;
        writeln!(buffer)?;
    }

    // write temp file and open it
    let file = temp_file::with_contents(&buffer);

    let output = Command::new(args.editor)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .arg(file.path())
        .output()?;

    if !output.status.success() {
        eprintln!("editor exited with: {}", output.status);
        return Ok(ExitCode::FAILURE);
    }

    // read temp file
    let mut after = vec![];
    let buffer = std::fs::read(file.path())?;
    for line in buffer.lines() {
        let line = line?;

        if line.is_empty() || line.starts_with("//") {
            continue;
        }

        after.push(line);
    }

    drop(file);

    if before.len() != after.len() {
        eprintln!(
            "had {} path(s) before, but {} path(s) after",
            before.len(),
            after.len(),
        );

        return Ok(ExitCode::FAILURE);
    }

    // rename files in reverse order
    let mut failure = false;

    let mut renamed_ancestors = BTreeSet::new();
    let mut reverse_map: BTreeMap<&str, Vec<&str>> = BTreeMap::new();

    for (before, after) in Iterator::zip(before.iter(), after.iter()) {
        let befores = reverse_map.entry(after).or_default();
        befores.push(before);

        if befores.len() > 1 {
            failure = true;
        }

        if let Some((before_stem, after_stem)) =
            Option::zip(get_ancestor(before), get_ancestor(after))
        {
            if before_stem != after_stem {
                failure = true;
                renamed_ancestors.insert((before_stem, after_stem));
            }
        }
    }

    // TODO: doesn't account for cross renames
    if reverse_map.values().any(|befores| befores.len() > 1) {
        eprintln!("duplicate renames:");
        for (after, befores) in reverse_map
            .into_iter()
            .filter(|(_, befores)| befores.len() > 1)
        {
            eprintln!("-> {after}");
            for before in befores {
                eprintln!("<- {before}");
            }
            eprintln!();
        }
    }

    if !renamed_ancestors.is_empty() {
        let pad = renamed_ancestors
            .iter()
            .map(|(b, _)| b.len())
            .max()
            .unwrap();

        eprintln!("inline renamed ancestors:");
        for (before, after) in renamed_ancestors {
            eprintln!("{before:<pad$} -> {after}");
        }
    }

    if failure {
        return Ok(ExitCode::FAILURE);
    }

    let pad = before.iter().map(|p| p.len()).max().unwrap();
    for (before, after) in Iterator::zip(before.iter().rev(), after.iter().rev()) {
        if before == after {
            continue;
        }

        eprintln!("{before:<pad$} -> {after}");
        if !args.dry_run {
            std::fs::rename(before, after)?
        }
    }

    Ok(ExitCode::SUCCESS)
}
