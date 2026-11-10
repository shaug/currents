mod checks;
mod quality;
mod readiness;
mod report;

fn main() {
    if let Err(error) = checks::run(std::env::args().skip(1).collect()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
