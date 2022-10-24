use nix::libc::{STDERR_FILENO, STDIN_FILENO, STDOUT_FILENO};
use nix::unistd;
use std::io::{Read, Write};
use std::os::unix::io::RawFd;

fn set_cloexec(fd: RawFd) {
    use nix::fcntl::{fcntl, FcntlArg, OFlag};
    let old_flags = OFlag::from_bits(fcntl(fd, FcntlArg::F_GETFL).expect("GETFL")).unwrap();
    let new_flags = old_flags | OFlag::O_CLOEXEC;
    fcntl(fd, FcntlArg::F_SETFL(new_flags)).expect("set O_CLOEXEC");
}

pub fn pipe_pair() -> (FdRead, FdWrite) {
    let (pipe_out, pipe_in) = unistd::pipe().expect("pipe");
    set_cloexec(pipe_out);
    set_cloexec(pipe_in);
    (FdRead(pipe_out), FdWrite(pipe_in))
}

#[derive(Debug, Clone, Copy)]
pub struct FdWrite(pub RawFd);

#[derive(Debug, Clone, Copy)]
pub struct FdRead(pub RawFd);

impl Write for FdWrite {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        unistd::write(self.0, buf).map_err(|err| std::io::Error::from_raw_os_error(err as i32))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Read for FdRead {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        unistd::read(self.0, buf).map_err(|err| std::io::Error::from_raw_os_error(err as i32))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Io {
    pub input: FdRead,
    pub output: FdWrite,
    pub error: FdWrite,
}

impl Io {
    pub fn stdio() -> Self {
        Self {
            input: FdRead(STDIN_FILENO),
            output: FdWrite(STDOUT_FILENO),
            error: FdWrite(STDERR_FILENO),
        }
    }

    pub fn set_input(mut self, input: FdRead) -> Self {
        self.input = input;
        self
    }

    pub fn set_output(mut self, output: FdWrite) -> Self {
        self.output = output;
        self
    }

    pub fn set_error(mut self, error: FdWrite) -> Self {
        self.error = error;
        self
    }
}
