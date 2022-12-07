mod ast;
mod builtins;
mod io;

use nix::errno::Errno;
use nix::libc::{STDERR_FILENO, STDIN_FILENO, STDOUT_FILENO};
use nix::sys::{signal, termios, wait};
use nix::unistd::{self, Pid};
use std::collections::HashMap;
use std::ffi::{CStr, CString, OsStr, OsString};
use std::io::Read;
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};

use crate::terminal_size;
use ast::*;
use io::{pipe_pair, Io};

fn str_c_to_os(cstr: &CStr) -> &OsStr {
    OsStr::from_bytes(cstr.to_bytes())
}
fn str_r_to_os(s: &str) -> &OsStr {
    OsStr::new(s)
}

fn get_termios() -> Result<termios::Termios, Errno> {
    termios::tcgetattr(STDIN_FILENO)
}
fn set_termios(termios: &termios::Termios) -> Result<(), Errno> {
    termios::tcsetattr(STDIN_FILENO, termios::SetArg::TCSANOW, termios)
}

pub fn expand_tilde(bytes: &[u8]) -> Vec<u8> {
    if bytes.first() == Some(&b'~') {
        let home = std::env::var_os("HOME").unwrap_or_else(|| todo!());

        let mut expanded = Vec::new();
        expanded.extend_from_slice(home.as_bytes());
        expanded.extend_from_slice(&bytes[1..]);
        expanded
    } else {
        bytes.to_vec()
    }
}

pub fn expand_pattern(bytes: &[u8]) -> Vec<u8> {
    if !bytes.contains(&b'*') {
        return bytes.to_vec();
    }

    type Stack<T> = Vec<T>;

    // split the bytes into parts by '/' and reverse them
    // example: "src/*.rs" --> ["*.rs", "src"]
    let mut patterns: Stack<OsString> = bytes
        .split(|b| *b == std::path::MAIN_SEPARATOR as u8)
        .map(|s| OsStr::from_bytes(s).to_owned())
        .rev()
        .collect();

    if let Some(pat) = patterns.last() {
        if pat.is_empty() {
            patterns.pop();
        }
    }

    let mut origin = if bytes.first().copied() == Some(std::path::MAIN_SEPARATOR as u8) {
        PathBuf::from("/")
    } else {
        PathBuf::from(".")
    };

    fn search(matched: &mut Vec<PathBuf>, dir: &mut PathBuf, patterns: &mut Stack<OsString>) {
        let pat = patterns.pop().unwrap();

        let Ok(mut dirhandle) = nix::dir::Dir::open(
            dir,
            nix::fcntl::OFlag::O_DIRECTORY,
            nix::sys::stat::Mode::empty(),
        ) else { return };

        for ent in dirhandle.iter().filter_map(|ent| ent.ok()) {
            let file_name = OsStr::from_bytes(ent.file_name().to_bytes());

            if !matches(pat.as_bytes(), file_name.as_bytes()) {
                continue;
            }
            let Some(ft) = ent.file_type() else { continue };

            let mut dent_path = dir.clone();
            dent_path.push(file_name);

            let is_dir = if let nix::dir::Type::Symlink = ft {
                // retrieve the metadata of the file pointed to by the symlink
                match std::fs::metadata(&dent_path) {
                    Ok(meta) => meta.is_dir(),
                    Err(_) => false, // treat this file as a regular file
                }
            } else {
                matches!(ft, nix::dir::Type::Directory)
            };

            if patterns.is_empty() {
                // if we have no more pattern, it means this path can be matched against the pattern.
                matched.push(dent_path);
            } else if is_dir {
                // if the current entry is a directory, continue searching over there.
                dir.push(file_name);
                search(matched, dir, patterns);
                dir.pop();
            }
        }

        patterns.push(pat);
    }

    fn matches(pat: &[u8], name: &[u8]) -> bool {
        match (pat, name) {
            ([], []) => true,
            ([], _) => false,
            ([b'*', pat_tail @ ..], name) => {
                for i in 0..name.len() + 1 {
                    if matches(pat_tail, &name[i..]) {
                        return true;
                    }
                }
                false
            }
            ([ch1, pat_tail @ ..], [ch2, name_tail @ ..]) => {
                if ch1 == ch2 {
                    matches(pat_tail, name_tail)
                } else {
                    false
                }
            }
            (_, []) => false,
        }
    }

    let mut matched = Vec::new();
    search(&mut matched, &mut origin, &mut patterns);

    let mut ret = Vec::new();
    for path in matched {
        ret.extend(path.as_os_str().as_bytes());
        ret.push(b' ');
    }
    ret.pop();
    ret
}

type Pgid = Pid;

#[derive(Clone)]
enum Executable {
    External(PathBuf),
    Builtin(fn(shell: &mut Shell, args: &[CString], io: Io) -> i32),
}

#[derive(Debug)]
struct Job {
    interactive: bool,
    pgid: Option<Pgid>,
    members: HashMap<Pid, Process>,
    last_status: Option<i32>,
    saved_termios: Option<termios::Termios>,
}

impl Job {
    fn new(interactive: bool) -> Self {
        let pgid = if interactive {
            None
        } else {
            Some(unistd::getpgrp())
        };

        Job {
            interactive,
            pgid,
            members: HashMap::new(),
            last_status: None,
            saved_termios: None,
        }
    }

    fn is_stopped(&self) -> bool {
        self.members.values().all(|p| p.is_completed() || p.stopped)
    }

    fn is_completed(&self) -> bool {
        self.members.values().all(|p| p.is_completed())
    }
}

#[derive(Debug)]
struct Process {
    pid: Pid,
    stopped: bool,
    status: Option<i32>,
}

impl Process {
    fn is_completed(&self) -> bool {
        self.status.is_some()
    }
}

pub struct Shell {
    shell_pgid: Pgid,
    env: Env,
    jobs: HashMap<Pgid, Job>,

    cd_undo_stack: Vec<PathBuf>,
    cd_redo_stack: Vec<PathBuf>,
}

impl Shell {
    pub fn new() -> Self {
        use signal::{killpg, sigaction, SaFlags, SigAction, SigHandler, SigSet, Signal};

        let interactive = unistd::isatty(STDIN_FILENO).expect("isatty");
        assert!(interactive, "only interactive shell is supported for now");

        // Loop while we are in the background
        loop {
            let fg_pgid = unistd::tcgetpgrp(STDIN_FILENO).expect("tcgetpgrp");
            let shell_pgid = unistd::getpgrp();

            if fg_pgid == shell_pgid {
                break;
            }

            killpg(shell_pgid, Signal::SIGTTIN).expect("killpg");
        }

        let sigign = SigAction::new(SigHandler::SigIgn, SaFlags::empty(), SigSet::empty());
        unsafe { sigaction(Signal::SIGINT, &sigign).expect("sigaction SIGINT") };
        unsafe { sigaction(Signal::SIGQUIT, &sigign).expect("sigaction SIGQUIT") };
        unsafe { sigaction(Signal::SIGTSTP, &sigign).expect("sigaction SIGTSTP") };
        unsafe { sigaction(Signal::SIGTTOU, &sigign).expect("sigaction SIGTTOU") };
        unsafe { sigaction(Signal::SIGTTIN, &sigign).expect("sigaction SIGTTIN") };

        let sigdfl = SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::empty());
        unsafe { sigaction(Signal::SIGCHLD, &sigdfl).expect("sigaction SIGCHLD") };
        unsafe { sigaction(Signal::SIGPIPE, &sigdfl).expect("sigaction SIGPIPE") };

        let pid = unistd::getpid();
        match unistd::setpgid(pid, pid) {
            Ok(()) => {}
            Err(Errno::EPERM) => {
                // this process is a session-leader
                // NOTE: this case will happen when another shell process is replaced by execve(2)
            }
            Err(err) => {
                panic!("{err}");
            }
        }
        let _ = unistd::setpgid(pid, pid);
        let shell_pgid = pid;
        unistd::tcsetpgrp(STDIN_FILENO, shell_pgid).expect("tcsetpgrp");

        let mut env = Env::new();
        if let Ok(cwd) = std::env::current_dir() {
            env.set_env("PWD", cwd.into_os_string());
        }

        Self {
            shell_pgid,
            env,
            jobs: HashMap::new(),

            cd_undo_stack: Vec::new(),
            cd_redo_stack: Vec::new(),
        }
    }

    pub fn jobs(&self) -> usize {
        self.jobs.len()
    }

    fn wait_for_job(&mut self, job_pgid: Pgid) -> i32 {
        if let Some(job) = self.jobs.get(&job_pgid) {
            if job.members.is_empty() {
                let status = job.last_status.unwrap();
                if job.is_completed() {
                    self.jobs.remove(&job_pgid);
                } else {
                    unreachable!();
                }
                return status;
            }
        }

        loop {
            let child_any = Pid::from_raw(-1);
            let handle_stop = Some(wait::WaitPidFlag::WUNTRACED);
            let wait_status = wait::waitpid(child_any, handle_stop).expect("waitpid");

            self.mark_process_status(wait_status);

            let job = self.jobs.get(&job_pgid).unwrap();
            if job.is_stopped() || job.is_completed() {
                let status = job.last_status.unwrap();
                if job.is_completed() {
                    self.jobs.remove(&job_pgid);
                }
                return status;
            }
        }
    }

    fn mark_process_status(&mut self, wait_status: wait::WaitStatus) {
        match wait_status {
            wait::WaitStatus::Exited(pid, status) => {
                // exited by _exit
                for job in self.jobs.values_mut() {
                    for p in job.members.values_mut() {
                        if p.pid == pid {
                            p.status = Some(status);
                            job.last_status = Some(status);
                            return;
                        }
                    }
                }
                unreachable!("procedd {pid} not found");
            }

            wait::WaitStatus::Signaled(pid, signal, _coredump) => {
                // killed by a signal
                for job in self.jobs.values_mut() {
                    for p in job.members.values_mut() {
                        if p.pid == pid {
                            // eprintln!("\x1b[7mprocess {pid} is terminated by {signal:?}\x1b[m");
                            let signaled = 128 + signal as i32;
                            p.status = Some(signaled);
                            job.last_status = Some(signaled);
                            return;
                        }
                    }
                }
                unreachable!("procedd {pid} not found");
            }

            wait::WaitStatus::Stopped(pid, signal) => {
                // stopped by a signal
                for job in self.jobs.values_mut() {
                    for p in job.members.values_mut() {
                        if p.pid == pid {
                            // eprintln!("\x1b[7mprocess {pid} is stopped by {signal:?}\x1b[m");
                            p.stopped = true;
                            let signaled = 128 + signal as i32;
                            job.last_status = Some(signaled);
                            return;
                        }
                    }
                }
                unreachable!("procedd {pid} not found");
            }

            _ => unreachable!(),
        }
    }

    fn set_foreground(&mut self, pgid: Pgid) {
        unistd::tcsetpgrp(STDIN_FILENO, pgid).expect("tcsetpgrp");
    }

    pub fn eval(&mut self, program: &str) -> i32 {
        match ast::parser::toplevel(program) {
            Ok(program_tree) => self.eval_list(&program_tree, Io::stdio(), true),
            Err(_err) => {
                eprintln!("Syntax Error");
                127
            }
        }
    }

    fn eval_list(&mut self, list: &List, io: Io, interactive: bool) -> i32 {
        let mut last_status;

        {
            let mut job = Job::new(interactive);
            self.eval_pipeline(&list.first, &mut job, io);
            let job_pgid = job.pgid.unwrap();
            self.jobs.insert(job_pgid, job);

            let saved_termios = get_termios().expect("tcgetattr");

            self.set_foreground(job_pgid);
            last_status = self.wait_for_job(job_pgid);
            self.set_foreground(self.shell_pgid);

            if let Some(job) = self.jobs.get_mut(&job_pgid) {
                if job.is_stopped() {
                    job.saved_termios = Some(get_termios().expect("tcgetattr"));
                    set_termios(&saved_termios).expect("tcsetattr");
                }
            }
        }

        for (cond, pipeline) in list.following.iter() {
            if (*cond == Condition::IfSuccess && last_status != 0)
                || (*cond == Condition::IfError && last_status == 0)
            {
                break;
            }

            let mut job = Job::new(interactive);
            self.eval_pipeline(pipeline, &mut job, io);
            let job_pgid = job.pgid.unwrap();
            self.jobs.insert(job_pgid, job);

            let saved_termios = get_termios().expect("tcgetattr");

            self.set_foreground(job_pgid);
            last_status = self.wait_for_job(job_pgid);
            self.set_foreground(self.shell_pgid);

            if let Some(job) = self.jobs.get_mut(&job_pgid) {
                if job.is_stopped() {
                    job.saved_termios = Some(get_termios().expect("tcgetattr"));
                    set_termios(&saved_termios).expect("tcsetattr");
                }
            }
        }

        if !interactive {
            std::process::exit(last_status);
        }

        last_status
    }

    fn eval_pipeline(&mut self, pipeline: &Pipeline, job: &mut Job, io: Io) {
        match pipeline {
            Pipeline::Single(cmd) => {
                self.eval_command(cmd, job, io);
            }

            Pipeline::Connected { pipe, lhs, rhs } => {
                let (pipe_read, pipe_write) = pipe_pair();

                let lhs_io;
                let rhs_io;
                match pipe {
                    Pipe::Stdout => {
                        lhs_io = io.set_output(pipe_write);
                        rhs_io = io.set_input(pipe_read);
                    }
                    Pipe::Stderr => {
                        lhs_io = io.set_error(pipe_write);
                        rhs_io = io.set_input(pipe_read);
                    }
                    Pipe::Both => {
                        lhs_io = io.set_output(pipe_write).set_error(pipe_write);
                        rhs_io = io.set_input(pipe_read);
                    }
                }

                self.eval_pipeline(lhs, job, lhs_io);
                unistd::close(pipe_write.0).expect("close");

                self.eval_pipeline(rhs, job, rhs_io);
                unistd::close(pipe_read.0).expect("close");
            }
        }
    }

    fn eval_command(&mut self, cmd: &Command, job: &mut Job, io: Io) {
        match cmd {
            Command::Simple(args) => {
                let mut args: Vec<CString> = args.iter().flat_map(|a| self.eval_args(a)).collect();
                assert!(!args.is_empty());

                let arg0 = str_c_to_os(&args[0]);
                if let Some(alias_values) = self.env.aliases.get(arg0) {
                    let mut actual_args: Vec<CString> = alias_values
                        .iter()
                        .map(|s| CString::new(s.as_bytes()).unwrap())
                        .collect();
                    actual_args.extend(args.drain(1..));
                    std::mem::swap(&mut args, &mut actual_args);
                }

                let exe = {
                    let arg0_os = str_c_to_os(&args[0]);
                    self.env.commands.get(arg0_os).cloned().unwrap_or_else(|| {
                        let path = PathBuf::from(arg0_os);
                        Executable::External(path)
                    })
                };

                match exe {
                    Executable::External(exe_path) => self.do_fork_exec(&exe_path, &args, job, io),

                    Executable::Builtin(impl_fptr) => {
                        let status = impl_fptr(self, &args, io);
                        if job.pgid.is_none() {
                            job.pgid = Some(self.shell_pgid);
                        }
                        job.last_status = Some(status);
                    }
                }
            }

            Command::SubShell(_list) => {
                // TODO
                // 1. fork
                // 2. derive or assign pgid
                // 3. don't ignore SIGINT, SIGTSTP, SIGQUIT, SIGTTOU, SIGTTIN,
                // 4. wait for the children normally
                // 5. exit with last status

                // When the parent shell process waits for a job consists of a subshell,
                // - it waits for the forked shell process as if it was a single command
                // - the forked shell process waits for processes launched by that,
                //   and finally it would terminate with the last status code of the children

                // If a user stops foreground processes by hitting <CTRL-Z>,
                // the forked process will be stopped because:
                // - it belongs to the foreground process group
                // - it doesn't ignore the SIGTSTP signal

                // If a user terminates foreground processes by hitting <CTRL-C>,
                // the forked process will be terminated because:
                // - it belongs to the foreground process group
                // - it doesn't ignore the SIGINT signal
                todo!();
            }
        }
    }

    fn eval_args(&mut self, args: &Arguments) -> Vec<CString> {
        match args {
            Arguments::Arg(str_parts) => {
                let bytes = self.eval_str(str_parts);
                let cstring = CString::new(bytes).unwrap();
                vec![cstring]
            }

            Arguments::AtExpansion(s) => {
                self.eval_str(s)
                    .split(|&b| {
                        // FIXME: support other whitespace characters
                        b == b' ' || b == b'\n' || b == b'\t'
                    })
                    .filter(|chunk| !chunk.is_empty())
                    .map(|chunk| {
                        let bytes = chunk.to_vec();
                        CString::new(bytes).unwrap()
                    })
                    .collect()
            }
        }
    }

    fn eval_str(&mut self, parts: &[StrPart]) -> Vec<u8> {
        let mut buf = Vec::new();
        for part in parts {
            match part {
                StrPart::Chars(chars) => buf.extend(chars.as_bytes()),

                StrPart::Expansion(expansion) => match expansion {
                    Expansion::Variable { name } => {
                        let name = str_r_to_os(name);
                        if let Some(value) = self.env.shell_vars.get(name) {
                            buf.extend_from_slice(value.as_bytes());
                        } else if let Some(value) = self.env.env_vars.get(name) {
                            buf.extend_from_slice(value.as_bytes());
                        }
                    }

                    Expansion::SubstStdout(list)
                    | Expansion::SubstStderr(list)
                    | Expansion::SubstBoth(list) => {
                        let (pipe_read, pipe_write) = pipe_pair();

                        let io = match expansion {
                            Expansion::SubstStdout(_) => Io::stdio().set_output(pipe_write),
                            Expansion::SubstStderr(_) => Io::stdio().set_error(pipe_write),
                            Expansion::SubstBoth(_) => {
                                Io::stdio().set_output(pipe_write).set_error(pipe_write)
                            }
                            _ => unreachable!(),
                        };

                        let child = match unsafe { unistd::fork() } {
                            Ok(unistd::ForkResult::Child) => {
                                unistd::close(pipe_read.0).expect("close");

                                self.eval_list(list, io, false);
                                unreachable!();
                            }

                            Ok(unistd::ForkResult::Parent { child, .. }) => {
                                unistd::close(pipe_write.0).expect("close");
                                child
                            }

                            Err(_) => panic!("fork failed"),
                        };

                        let mut pipe_read = pipe_read;

                        // TODO: sysconf ARG_MAX
                        const ARG_SIZE_LIMIT: u64 = 0x200000;

                        let mut arg_buf = Vec::new();
                        (&mut pipe_read)
                            .take(ARG_SIZE_LIMIT)
                            .read_to_end(&mut arg_buf)
                            .expect("read");

                        unistd::close(pipe_read.0).expect("close");

                        wait::waitpid(child, None).expect("wait");

                        for byte in arg_buf {
                            if byte == b' ' || byte == b'\n' || byte == b'\t' {
                                if !matches!(buf.last(), Some(b' ')) {
                                    buf.push(b' ');
                                }
                            } else {
                                buf.push(byte);
                            }
                        }

                        if matches!(buf.last(), Some(b' ')) {
                            buf.pop();
                        }
                    }

                    Expansion::SubstPipeName(_list) => {
                        todo!();
                    }

                    Expansion::SubstStatus(_list) => {
                        todo!();
                    }
                },
            }
        }

        let buf = expand_tilde(&buf);
        let buf = expand_pattern(&buf);

        buf
    }

    fn do_fork_exec(&mut self, exe_path: &Path, args: &[CString], job: &mut Job, io: Io) {
        let exe = CString::new(exe_path.as_os_str().as_bytes()).unwrap();

        match unsafe { unistd::fork() } {
            Ok(unistd::ForkResult::Child) => {
                let current_pid = unistd::getpid();
                let pgid = job.pgid.unwrap_or(current_pid);
                unistd::setpgid(current_pid, pgid).expect("setpgid");
                unistd::tcsetpgrp(STDIN_FILENO, pgid).expect("tcsetpgrp");

                use signal::{sigaction, SaFlags, SigAction, SigHandler, SigSet, Signal};
                let sigdfl = SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::empty());
                unsafe { sigaction(Signal::SIGINT, &sigdfl).expect("sigaction") };
                unsafe { sigaction(Signal::SIGQUIT, &sigdfl).expect("sigaction") };

                if job.interactive {
                    unsafe { sigaction(Signal::SIGTSTP, &sigdfl).expect("sigaction") };
                    unsafe { sigaction(Signal::SIGTTIN, &sigdfl).expect("sigaction") };
                    unsafe { sigaction(Signal::SIGTTOU, &sigdfl).expect("sigaction") };
                }

                unistd::dup2(io.input.0, STDIN_FILENO).expect("dup2");
                unistd::dup2(io.output.0, STDOUT_FILENO).expect("dup2");
                unistd::dup2(io.error.0, STDERR_FILENO).expect("dup2");

                let envs: Vec<CString> = self
                    .env
                    .env_vars
                    .iter()
                    .map(|(k, v)| {
                        let k = k.as_bytes();
                        let v = v.as_bytes();

                        let mut buf = Vec::with_capacity(k.len() + 1 + v.len());
                        buf.extend_from_slice(k);
                        buf.push(b'=');
                        buf.extend_from_slice(v);

                        CString::new(buf).unwrap()
                    })
                    .collect();

                match unistd::execve(&exe, args, &envs) {
                    Ok(_) => unreachable!(),
                    Err(Errno::ENOENT) => {
                        std::process::exit(127);
                    }
                    Err(_) => {
                        std::process::exit(126);
                    }
                }
            }

            Ok(unistd::ForkResult::Parent { child, .. }) => {
                let pgid = job.pgid.unwrap_or(child);
                match unistd::setpgid(child, pgid) {
                    Ok(()) => {}
                    Err(Errno::EACCES) => {
                        // ignore this error
                    }
                    Err(err) => {
                        panic!("setpgid: {err}");
                    }
                }

                let process = Process {
                    pid: child,
                    stopped: false,
                    status: None,
                };

                job.pgid = Some(pgid);
                job.members.insert(child, process);
            }

            Err(_) => panic!("fork failed"),
        }
    }

    pub fn list_commands(&self) -> Vec<String> {
        self.env
            .commands
            .keys()
            .filter_map(|os| Some(std::str::from_utf8(os.as_bytes()).ok()?.to_owned()))
            .collect()
    }

    pub fn update_variables(&mut self) {
        let nrows = terminal_size::get_rows();
        let nrows = OsString::from(nrows.to_string());
        self.env.set_env("LINES", nrows);

        let ncols = terminal_size::get_cols();
        let ncols = OsString::from(ncols.to_string());
        self.env.set_env("COLUMNS", ncols);
    }
}

#[derive(Clone)]
pub struct Env {
    aliases: HashMap<OsString, Vec<OsString>>,
    commands: HashMap<OsString, Executable>,
    env_vars: HashMap<OsString, OsString>,
    shell_vars: HashMap<OsString, OsString>,
}

impl Env {
    pub fn new() -> Self {
        let mut env = Env {
            aliases: HashMap::new(),
            commands: HashMap::new(),
            env_vars: std::env::vars_os().collect(),
            shell_vars: HashMap::new(),
        };

        env.update_commands();
        env
    }

    pub fn update_commands(&mut self) {
        self.commands.clear();

        let path_value = match self.get_env("PATH") {
            Some(val) => val.to_owned(),
            None => {
                return;
            }
        };

        for path in std::env::split_paths(&path_value) {
            let entries = match std::fs::read_dir(&path) {
                Ok(ents) => ents,
                Err(_err) => {
                    // eprintln!("{err}");
                    continue;
                }
            };

            for ent in entries {
                let ent = match ent {
                    Ok(e) => e,
                    Err(_err) => {
                        // eprintln!("{err}");
                        continue;
                    }
                };

                if ent.file_type().map(|ty| ty.is_dir()).unwrap_or(true) {
                    continue;
                }

                let basename = ent.file_name();
                let path = ent.path();
                // eprintln!("{:?} => {:?}", basename, path);
                self.commands
                    .entry(basename)
                    .or_insert(Executable::External(path));
            }
        }

        // register builtin commands
        {
            macro_rules! builtin_bind {
                ($cmd:expr, $impl_name:path) => {{
                    let tmp = Executable::Builtin($impl_name);
                    self.commands.insert($cmd.into(), tmp);
                }};
            }

            use builtins::*;
            builtin_bind!("args", builtin_args);
            builtin_bind!("exit", builtin_exit);
            builtin_bind!("cd", builtin_cd);
            builtin_bind!("jobs", builtin_jobs);
            builtin_bind!("fg", builtin_fg);
            builtin_bind!(">>", builtin_append);
            builtin_bind!(">", builtin_overwrite);
            builtin_bind!("alias", builtin_alias);
            builtin_bind!("var", builtin_var);
            builtin_bind!("evar", builtin_evar);
            builtin_bind!("unset", builtin_unset);
        }
    }

    pub fn get_env<'a>(&self, name: &'a str) -> Option<&'_ OsStr> {
        self.env_vars
            .get(str_r_to_os(name))
            .map(|val| val.as_os_str())
    }

    pub fn set_env(&mut self, name: &str, value: OsString) {
        self.env_vars.insert(str_r_to_os(name).to_owned(), value);
    }
}
