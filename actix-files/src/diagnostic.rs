use std::{
    fmt,
    io::{self, Write as _},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    thread,
};

const ENABLE_ENV: &str = "ACTIX_FILES_IO_URING_DIAGNOSTICS";
const PATH_METADATA_ENV: &str = "ACTIX_FILES_IO_URING_USE_PATH_METADATA";

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static FOCUSED: AtomicBool = AtomicBool::new(false);

pub(crate) fn enabled() -> bool {
    cfg!(test) && std::env::var_os(ENABLE_ENV).is_some()
}

pub(crate) fn use_path_metadata() -> bool {
    cfg!(test) && matches!(std::env::var(PATH_METADATA_ENV).as_deref(), Ok("1"))
}

pub(crate) fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
pub(crate) fn focus() {
    FOCUSED.store(true, Ordering::Relaxed);
}

pub(crate) fn log(args: fmt::Arguments<'_>) {
    if !enabled() || !FOCUSED.load(Ordering::Relaxed) {
        return;
    }

    let mut stderr = io::stderr().lock();
    let _ = writeln!(
        stderr,
        "[DEBUG-io-uring] thread={:?} {args}",
        thread::current().id()
    );
    let _ = stderr.flush();
}
