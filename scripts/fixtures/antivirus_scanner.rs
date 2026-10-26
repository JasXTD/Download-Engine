// Harmless scanner protocol fixture. Compile only for local integration tests;
// never include this executable in a product package.
use std::{env, fs, process, thread, time::Duration};
fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() != 3 { process::exit(2); }
    let bytes = fs::read(&args[2]).expect("Scanner must receive one literal, readable file path");
    assert!(!bytes.is_empty(), "Fixture must contain downloaded bytes");
    match args[1].as_str() {
        "clean" => { println!("Fixture scanned {} bytes; no threats simulated", bytes.len()); }
        "detected" => { println!("Simulated detection; this file is harmless"); process::exit(1); }
        "error" => { eprintln!("Simulated scanner failure"); process::exit(2); }
        "timeout" => thread::sleep(Duration::from_secs(30)),
        _ => process::exit(2),
    }
}
