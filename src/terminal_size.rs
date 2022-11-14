#![allow(unused)]

use std::sync::atomic::{AtomicU16, Ordering};
static ROWS: AtomicU16 = AtomicU16::new(0);
static COLS: AtomicU16 = AtomicU16::new(0);

pub fn get_rows() -> u16 {
    ROWS.load(Ordering::SeqCst)
}

pub fn get_cols() -> u16 {
    COLS.load(Ordering::SeqCst)
}

pub fn update() {
    let mut winsize = nix::pty::Winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };

    nix::ioctl_read_bad!(tiocgwinsz, nix::libc::TIOCGWINSZ, nix::pty::Winsize);
    unsafe { tiocgwinsz(0, &mut winsize as *mut nix::pty::Winsize) }.expect("ioctl");

    ROWS.store(winsize.ws_row, Ordering::SeqCst);
    COLS.store(winsize.ws_col, Ordering::SeqCst);
}

/// Installs a signal handler for SIGWINCH
pub fn install_sigwinch_handler() {
    update();

    use nix::sys::signal::{sigaction, SaFlags, SigAction, SigHandler, SigSet, Signal};
    let handler = SigHandler::Handler(sigwinch_handler);
    let action = SigAction::new(handler, SaFlags::empty(), SigSet::empty());
    unsafe { sigaction(Signal::SIGWINCH, &action).expect("sigaction for SIGWINCH") };
}

extern "C" fn sigwinch_handler(_: i32) {
    update();
}
