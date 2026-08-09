//! Newline-framed local model fixture used by process transport contracts.

use std::io::{BufRead as _, Write as _};
use std::time::Duration;

fn main() {
    let mut request = String::new();
    std::io::stdin().lock().read_line(&mut request).unwrap();
    assert!(request.contains("private-input"));
    println!(r#"{{"token":"process-first"}}"#);
    std::io::stdout().flush().unwrap();
    std::thread::sleep(Duration::from_millis(30));
    println!(r#"{{"token":"process-second"}}"#);
}
