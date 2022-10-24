use nix::sys::signal;
use nix::unistd::Pid;
use std::ffi::{CString, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::{get_termios, set_termios, str_c_to_os, str_r_to_os, Io, Pgid, Shell};

pub fn builtin_exit(shell: &mut Shell, _args: &[CString], mut io: Io) -> i32 {
    if shell.jobs.is_empty() {
        std::process::exit(0);
    } else {
        let _ = writeln!(
            &mut io.error,
            "exit: you have {} pending jobs.",
            shell.jobs.len()
        );
        1
    }
}

pub fn builtin_cd(shell: &mut Shell, args: &[CString], mut io: Io) -> i32 {
    enum Op {
        Undo,
        Redo,
        Chdir(PathBuf),
    }

    let op = match args.get(1) {
        None => {
            let home = shell
                .env
                .get_env("HOME")
                .unwrap_or_else(|| str_r_to_os("."));
            Op::Chdir(Path::new(home).to_owned())
        }

        Some(arg1) if arg1.as_bytes() == b"-" => Op::Undo,
        Some(arg1) if arg1.as_bytes() == b"+" => Op::Redo,
        Some(arg1) => Op::Chdir(Path::new(str_c_to_os(arg1)).to_owned()),
    };

    let old_cwd = std::env::current_dir();

    match op {
        Op::Undo => {
            if let Some(new_cwd) = shell.cd_undo_stack.pop() {
                if let Ok(old_cwd) = old_cwd {
                    shell.env.set_env("OLDPWD", old_cwd.as_os_str().to_owned());
                    shell.cd_redo_stack.push(old_cwd);
                }

                match std::env::set_current_dir(&new_cwd) {
                    Err(err) => {
                        let _ = writeln!(&mut io.error, "cd: {err}");
                        1
                    }
                    Ok(_) => {
                        shell.env.set_env("PWD", new_cwd.into_os_string());
                        0
                    }
                }
            } else {
                2
            }
        }

        Op::Redo => {
            if let Some(new_cwd) = shell.cd_redo_stack.pop() {
                if let Ok(old_cwd) = old_cwd {
                    shell.env.set_env("OLDPWD", old_cwd.as_os_str().to_owned());
                    shell.cd_undo_stack.push(old_cwd);
                }

                match std::env::set_current_dir(&new_cwd) {
                    Err(err) => {
                        let _ = writeln!(&mut io.error, "cd: {err}");
                        1
                    }
                    Ok(_) => {
                        shell.env.set_env("PWD", new_cwd.into_os_string());
                        0
                    }
                }
            } else {
                2
            }
        }

        Op::Chdir(new_cwd) => match std::env::set_current_dir(&new_cwd) {
            Err(err) => {
                let _ = writeln!(&mut io.error, "cd: {err}");
                1
            }

            Ok(_) => {
                if let Ok(old_cwd) = old_cwd {
                    shell.env.set_env("OLDPWD", old_cwd.as_os_str().to_owned());
                    shell.cd_undo_stack.push(old_cwd);
                }
                shell.env.set_env("PWD", new_cwd.into_os_string());
                shell.cd_redo_stack.clear();
                0
            }
        },
    }
}

pub fn builtin_jobs(shell: &mut Shell, _args: &[CString], mut io: Io) -> i32 {
    for (i, (pgid, _)) in shell.jobs.iter().enumerate() {
        let _ = writeln!(&mut io.output, "[{i}] {pgid}");
    }
    0
}

pub fn builtin_fg(shell: &mut Shell, args: &[CString], mut io: Io) -> i32 {
    let job_pgid = if let Some(arg) = args.get(1) {
        // CStr --> str --> i32 --> Pgid (Pid)
        let valid_pgid = std::str::from_utf8(arg.as_bytes())
            .ok()
            .and_then(|s| s.parse::<i32>().ok())
            .map(Pgid::from_raw)
            .and_then(|pgid| {
                if shell.jobs.contains_key(&pgid) {
                    Some(pgid)
                } else {
                    None
                }
            });

        if let Some(pgid) = valid_pgid {
            pgid
        } else {
            let _ = writeln!(&mut io.error, "fg: no such job is found");
            let _ = writeln!(&mut io.error, "fg: usage: fg <pgid>");
            return 1;
        }
    } else {
        match shell.jobs.iter().find(|(_, j)| j.is_stopped()) {
            Some((pgid, _)) => *pgid,
            None => {
                let _ = writeln!(&mut io.error, "fg: you have no suspended job");
                return 1;
            }
        }
    };

    let job = shell.jobs.get_mut(&job_pgid).unwrap();
    let saved_termios = get_termios().expect("tcgetattr");
    let job_termios = job.saved_termios.take().expect("not a suspended job");
    set_termios(&job_termios).expect("tcsetattr");

    shell.set_foreground(job_pgid);

    let status = {
        let job = shell.jobs.get_mut(&job_pgid).unwrap();
        for p in job.members.values_mut() {
            p.stopped = false;
        }

        let group_members = Pid::from_raw(-job_pgid.as_raw());
        signal::kill(group_members, signal::Signal::SIGCONT).expect("kill");

        shell.wait_for_job(job_pgid)
    };

    shell.set_foreground(shell.shell_pgid);

    if let Some(job) = shell.jobs.get_mut(&job_pgid) {
        if job.is_stopped() {
            job.saved_termios = Some(get_termios().expect("tcgetattr"));
            set_termios(&saved_termios).expect("tcsetattr");
        }
    }

    status
}

pub fn builtin_append(_shell: &mut Shell, args: &[CString], mut io: Io) -> i32 {
    let outpath = match args.get(1) {
        Some(arg) => Path::new(str_c_to_os(arg)),
        None => {
            let _ = writeln!(&mut io.error, ">>: takes 1 argument");
            return 1;
        }
    };

    let open_result = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(outpath);

    match open_result {
        Err(err) => {
            let _ = writeln!(&mut io.error, ">>: {err}");
            2
        }

        Ok(mut outfile) => {
            let mut input_pipe = io.input;
            if let Err(err) = std::io::copy(&mut input_pipe, &mut outfile) {
                let _ = writeln!(&mut io.error, ">>: {err}");
                3
            } else {
                0
            }
        }
    }
}

pub fn builtin_overwrite(_shell: &mut Shell, args: &[CString], mut io: Io) -> i32 {
    let outpath = match args.get(1) {
        Some(arg) => Path::new(str_c_to_os(arg)),
        None => {
            let _ = writeln!(&mut io.error, ">: takes 1 argument");
            return 1;
        }
    };

    let open_result = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(outpath);

    match open_result {
        Err(err) => {
            let _ = writeln!(&mut io.error, ">: {err}");
            2
        }

        Ok(mut outfile) => {
            let mut input_pipe = io.input;
            if let Err(err) = std::io::copy(&mut input_pipe, &mut outfile) {
                let _ = writeln!(&mut io.error, ">: {err}");
                3
            } else {
                0
            }
        }
    }
}

pub fn builtin_alias(shell: &mut Shell, args: &[CString], mut io: Io) -> i32 {
    debug_assert!(!args.is_empty());

    if args.len() == 1 {
        // % alias
        for (alias, values) in shell.env.aliases.iter() {
            println!("{:?} => {:?}", alias, values);
        }
        return 0;
    } else if args.len() >= 4 && args[2].as_bytes() == b"=" {
        // % alias foo = bar ...
        let name = str_c_to_os(&args[1]).to_owned();
        let values: Vec<OsString> = args[3..]
            .iter()
            .map(|c| str_c_to_os(c).to_owned())
            .collect();
        shell.env.aliases.insert(name, values);
        return 0;
    }

    let _ = writeln!(&mut io.error, "alias: invalid assignment");
    1
}

pub fn builtin_var(shell: &mut Shell, args: &[CString], mut io: Io) -> i32 {
    debug_assert!(!args.is_empty());

    if args.len() == 1 {
        for (key, val) in shell.env.shell_vars.iter() {
            println!("{:?} => {:?}", key, val);
        }
        return 0;
    } else if args.len() == 4 && args[2].as_bytes() == b"=" {
        let key = str_c_to_os(&args[1]).to_owned();
        let val = str_c_to_os(&args[3]).to_owned();
        shell.env.shell_vars.insert(key, val);
        return 0;
    }

    let _ = writeln!(&mut io.error, "var: invalid assignment");
    1
}

pub fn builtin_export(shell: &mut Shell, args: &[CString], mut io: Io) -> i32 {
    debug_assert!(!args.is_empty());

    let mut status = 0;
    for arg in args[1..].iter() {
        let name = str_c_to_os(arg);
        if let Some(value) = shell.env.shell_vars.get(name) {
            shell.env.env_vars.insert(name.to_owned(), value.to_owned());
        } else {
            let _ = writeln!(&mut io.error, "export: variable {:?} is undefined", name);
            status = 1;
        }
    }
    status
}
