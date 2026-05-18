use roder_configure::headless;

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let result = headless::run(
        &args,
        std::env::current_dir().unwrap_or_else(|_| ".".into()),
    );
    print!("{}", result.stdout);
    eprint!("{}", result.stderr);
    if result.status != 0 {
        std::process::exit(result.status);
    }
}
