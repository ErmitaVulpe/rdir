use std::os::fd::AsFd;

use nix::{
    libc,
    unistd::{ForkResult, fork, setsid},
};
use tracing::{error, level_filters::LevelFilter};
use tracing_appender::non_blocking::WorkerGuard;

use crate::{
    args::Args,
    server::{DOWNLOAD_CACHE_DIR, LOGS_DIR, LOGS_PREFIX},
};

/// Must be called just once
pub unsafe fn setup(args: &Args) -> anyhow::Result<WorkerGuard> {
    unsafe { daemonize(args)? };
    let guard = init_logs();
    let _ = std::fs::create_dir(DOWNLOAD_CACHE_DIR);
    Ok(guard)
}

unsafe fn daemonize(args: &Args) -> anyhow::Result<()> {
    // Fork again to prevent terminal re-acquisition
    match unsafe { fork()? } {
        ForkResult::Parent { .. } => std::process::exit(0),
        ForkResult::Child => {}
    }

    // Detach from terminal
    setsid()?;

    // Change working directory
    std::env::set_current_dir(&args.tmp_dir)?;

    // Close standard fds
    for fd in 0..3 {
        unsafe { libc::close(fd) };
    }

    // Redirect stdin, stdout, stderr to /dev/null
    let devnull = std::fs::File::open("/dev/null")?;
    let devnull_fd = devnull.as_fd();
    let _ = nix::unistd::dup2_stdin(devnull_fd);
    let _ = nix::unistd::dup2_stdout(devnull_fd);
    let _ = nix::unistd::dup2_stderr(devnull_fd);

    Ok(())
}

fn init_logs() -> WorkerGuard {
    let file_appender = tracing_appender::rolling::daily(LOGS_DIR, LOGS_PREFIX);
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_max_level(LevelFilter::DEBUG)
        .with_writer(non_blocking)
        .init();
    std::panic::set_hook(Box::new(move |panic_info| {
        error!(
            message = %panic_info,
            "panic occurred"
        );
    }));

    guard
}

pub fn clean_up(args: &Args) {
    // Tried removing "." but that left an empty dir
    let _ = std::fs::remove_dir_all(&args.tmp_dir);
}
