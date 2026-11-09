//! Windowless tracker process. GUI subsystem, so starting it (from
//! `screentime run` or the login Run key) never flashes a console window.
#![windows_subsystem = "windows"]

#[cfg(windows)]
fn main() {
    use screentime_core::daemon;
    match daemon::run() {
        Ok(daemon::Exit::Stopped) => {}
        Ok(daemon::Exit::AlreadyRunning) => std::process::exit(2),
        Err(e) => {
            daemon::log(&e);
            std::process::exit(1);
        }
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("screentime-tracker only runs on Windows");
    std::process::exit(1);
}
