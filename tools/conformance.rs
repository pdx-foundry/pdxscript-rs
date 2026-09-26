use pdxscript::script::*;
mod normalized;
use serde_json::{Value as Json, json};
use std::io::{self, BufRead, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut output = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let request: Json = serde_json::from_str(&line?)?;
        let source = request["source"].as_str().ok_or("source required")?;
        let file = request["file"].as_str().unwrap_or("<test>");
        let answer = match parse(source, file) {
            Ok(doc) => match normalized::response(&doc) {
                Ok(response) => response,
                Err(e) => json!({"write_error":e.to_string()}),
            },
            Err(e) => json!({"error":true,"line":e.span.line}),
        };
        serde_json::to_writer(&mut output, &answer)?;
        writeln!(output)?;
        output.flush()?;
    }
    Ok(())
}
