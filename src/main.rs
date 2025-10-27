use std::process::ExitCode;

fn main() -> ExitCode {
    if let Err(error) = revdeprun::run() {
        eprintln!("revdeprun: {error:?}");
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
