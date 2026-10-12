//! Times `parse_tape`, `parse`, and jomini's `TextTape` over an installed game's script files.
//!
//! `STELLARIS_PATH=/path/to/Stellaris cargo run --release` from this directory. Only UTF-8 files
//! under `common/` and `events/` that both parsers accept are timed, so both do the same work.
//! Each figure is the fastest of `ITERS` (default 20) passes over every file.
use std::{
    hint::black_box,
    path::Path,
    time::{Duration, Instant},
};

fn collect(dir: &Path, files: &mut Vec<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for path in entries.flatten().map(|entry| entry.path()) {
        if path.is_dir() {
            collect(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "txt")
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            files.push((path.display().to_string(), text));
        }
    }
}

fn fastest(iterations: usize, mut pass: impl FnMut()) -> Duration {
    pass();
    (0..iterations)
        .map(|_| {
            let start = Instant::now();
            pass();
            start.elapsed()
        })
        .min()
        .expect("at least one iteration")
}

fn main() {
    let install = std::env::var("STELLARIS_PATH").expect("STELLARIS_PATH is required");
    let iterations = std::env::var("ITERS").map_or(20, |n| n.parse().expect("ITERS is a number"));
    let mut files = Vec::new();
    for directory in ["common", "events"] {
        collect(&Path::new(&install).join(directory), &mut files);
    }
    files.retain(|(name, text)| {
        pdxscript::script::parse_tape(text, name).is_ok()
            && jomini::TextTape::from_slice(text.as_bytes()).is_ok()
    });
    assert!(!files.is_empty(), "no script files under {install}");
    let bytes: usize = files.iter().map(|(_, text)| text.len()).sum();
    println!("{} files, {:.1} MB", files.len(), bytes as f64 / 1e6);

    let jomini = fastest(iterations, || {
        for (_, text) in &files {
            black_box(jomini::TextTape::from_slice(text.as_bytes()).unwrap());
        }
    });
    let tape = fastest(iterations, || {
        for (name, text) in &files {
            black_box(pdxscript::script::parse_tape(text, name).unwrap());
        }
    });
    let document = fastest(iterations, || {
        for (name, text) in &files {
            black_box(pdxscript::script::parse(text, name).unwrap());
        }
    });
    for (label, time) in [
        ("jomini TextTape", jomini),
        ("pdxscript parse_tape", tape),
        ("pdxscript parse", document),
    ] {
        println!(
            "{label:<22}{:>9.2} ms{:>9.1} MB/s{:>7.2}x jomini",
            time.as_secs_f64() * 1e3,
            bytes as f64 / 1e6 / time.as_secs_f64(),
            time.as_secs_f64() / jomini.as_secs_f64(),
        );
    }
}
