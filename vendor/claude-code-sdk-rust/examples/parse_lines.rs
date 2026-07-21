use claude_code_sdk_rust::internal::parser::parse_message_line;

fn main() {
    let path = std::env::args().nth(1).expect("path to jsonl");
    for (idx, line) in std::fs::read_to_string(path).unwrap().lines().enumerate() {
        match parse_message_line(line) {
            Ok(Some(_)) => println!("{idx}: ok"),
            Ok(None) => println!("{idx}: skipped"),
            Err(err) => println!("{idx}: ERROR {err}"),
        }
    }
}
