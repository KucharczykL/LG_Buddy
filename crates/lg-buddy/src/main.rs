use std::env;
use std::process::ExitCode;

use lg_buddy::{help, parse_args, run_command, version, ParseOutcome};

fn main() -> ExitCode {
    let program = env::args().next().unwrap_or_else(|| "lg-buddy".to_string());

    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments.first().is_some_and(|arg| arg == "kwin-setup") {
        return ExitCode::from(lg_buddy::setup::run_kwin_setup(&arguments[1..]));
    }

    if arguments
        .first()
        .is_some_and(|arg| arg == "service-setup-ownership")
    {
        let path = std::path::Path::new(
            arguments
                .get(1)
                .map(String::as_str)
                .unwrap_or("/usr/bin/lg-buddy"),
        );
        if arguments.len() > 2 || !path.is_absolute() {
            return ExitCode::from(2);
        }
        return match lg_buddy::setup::service_setup_ownership(path) {
            Ok(ownership) => {
                println!("{ownership}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("LG Buddy: {error}");
                ExitCode::FAILURE
            }
        };
    }

    if arguments.first().is_some_and(|arg| arg == "config-binding") {
        if arguments.get(1).is_some_and(|arg| arg == "migrate") {
            // This installer entrypoint is single-threaded. Publish newly
            // created root directories with traversable permissions immediately,
            // including if installation stops before the binding is written.
            unsafe {
                libc::umask(0o022);
            }
        }
        return match lg_buddy::config::binding::run_installer_command(&arguments[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("LG Buddy: {error}");
                ExitCode::FAILURE
            }
        };
    }

    match parse_args(arguments) {
        Ok(ParseOutcome::Help(topic)) => {
            print!("{}", help(&program, topic));
            ExitCode::SUCCESS
        }
        Ok(ParseOutcome::Version) => {
            print!("{}", version::version_text());
            ExitCode::SUCCESS
        }
        Ok(ParseOutcome::Command(command)) => match run_command(command, &mut std::io::stdout()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("LG Buddy: {err}");
                ExitCode::from(match err {
                    lg_buddy::RunError::Setup(error) => error.exit_code(),
                    lg_buddy::RunError::GnomeReadinessProbe(error) => error.exit_code(),
                    _ => 1,
                })
            }
        },
        Err(err) => {
            let topic = err.help_topic();
            eprintln!("LG Buddy: {err}");
            eprintln!();
            eprint!("{}", help(&program, topic));
            ExitCode::from(2)
        }
    }
}
