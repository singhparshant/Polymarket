use std::fs::OpenOptions;
use std::io::Write;

// Minimal helper to mirror stdout messages into output.txt
pub fn logln(message: String) {
    // Always print to stdout (existing behavior)
    println!("{}", message);

    // Also append to output.txt
    if let Err(err) = append_to_file("output.txt", &message) {
        // Keep failures non-fatal; report to stderr
        eprintln!("logger: failed writing to output.txt: {:?}", err);
    }
}

fn append_to_file(path: &str, line: &str) -> std::io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}
