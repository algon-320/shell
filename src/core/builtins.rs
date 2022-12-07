use nix::sys::signal;
use nix::unistd::Pid;
use std::ffi::{CString, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::io::Io;
use super::{get_termios, set_termios, str_c_to_os, str_r_to_os, Pgid, Shell};

pub fn builtin_args(_shell: &mut Shell, args: &[CString], mut io: Io) -> i32 {
    for (i, arg) in args.iter().enumerate().skip(1) {
        let _ = writeln!(&mut io.output, "{i}: {:?}", arg);
    }
    0
}

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
                let actual_new_cwd =
                    std::env::current_dir().expect("getcwd right after chdir should success");

                if let Ok(old_cwd) = old_cwd {
                    shell.env.set_env("OLDPWD", old_cwd.as_os_str().to_owned());
                    shell.cd_undo_stack.push(old_cwd);
                }
                shell.env.set_env("PWD", actual_new_cwd.into_os_string());
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
    match args {
        [_arg0, outpath] => {
            let outpath = Path::new(str_c_to_os(outpath));
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(outpath);

            file.and_then(|mut file| std::io::copy(&mut io.input, &mut file))
                .map(|_| 0)
                .unwrap_or_else(|err| {
                    let _ = writeln!(&mut io.error, ">>: {err}");
                    2
                })
        }

        _ => {
            let _ = writeln!(&mut io.error, ">>: takes 1 argument");
            1
        }
    }
}

pub fn builtin_overwrite(_shell: &mut Shell, args: &[CString], mut io: Io) -> i32 {
    match args {
        [_arg0, outpath] => {
            let outpath = Path::new(str_c_to_os(outpath));
            let file = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(outpath);

            file.and_then(|mut file| std::io::copy(&mut io.input, &mut file))
                .map(|_| 0)
                .unwrap_or_else(|err| {
                    let _ = writeln!(&mut io.error, ">: {err}");
                    2
                })
        }

        _ => {
            let _ = writeln!(&mut io.error, ">: takes 1 argument");
            1
        }
    }
}

pub fn builtin_alias(shell: &mut Shell, args: &[CString], mut io: Io) -> i32 {
    match args {
        [_arg0] => {
            for (alias, values) in shell.env.aliases.iter() {
                let _ = writeln!(&mut io.output, "{alias:?} => {values:?}");
            }
            0
        }

        [_arg0, name, eq, values @ ..] if eq.as_bytes() == b"=" && !values.is_empty() => {
            let name = str_c_to_os(name).to_owned();
            let values: Vec<OsString> = values.iter().map(|c| str_c_to_os(c).to_owned()).collect();
            shell.env.aliases.insert(name, values);
            0
        }

        _ => {
            let _ = writeln!(&mut io.error, "alias: invalid assignment");
            1
        }
    }
}

pub fn builtin_var(shell: &mut Shell, args: &[CString], mut io: Io) -> i32 {
    match args {
        [_arg0] => {
            for (key, val) in shell.env.shell_vars.iter() {
                let _ = writeln!(&mut io.output, "{key:?} => {val:?}");
            }
            0
        }

        [_arg0, key, eq, val] if eq.as_bytes() == b"=" => {
            let key = str_c_to_os(key).to_owned();
            let val = str_c_to_os(val).to_owned();
            shell.env.shell_vars.insert(key, val);
            0
        }

        _ => {
            let _ = writeln!(&mut io.error, "var: invalid assignment");
            1
        }
    }
}

pub fn builtin_evar(shell: &mut Shell, args: &[CString], mut io: Io) -> i32 {
    match args {
        [_arg0] => {
            for (key, val) in shell.env.env_vars.iter() {
                let _ = writeln!(&mut io.output, "{key:?} => {val:?}");
            }
            0
        }

        [_arg0, key, eq, val] if eq.as_bytes() == b"=" => {
            let key = str_c_to_os(key).to_owned();
            let val = str_c_to_os(val).to_owned();
            shell.env.env_vars.insert(key, val);
            0
        }

        _ => {
            let _ = writeln!(&mut io.error, "evar: invalid assignment");
            1
        }
    }
}

pub fn builtin_unset(shell: &mut Shell, args: &[CString], mut _io: Io) -> i32 {
    match args {
        [_arg0, names @ ..] => {
            for name in names {
                let name = str_c_to_os(name);
                shell.env.env_vars.remove(name);
                shell.env.shell_vars.remove(name);
            }
            0
        }

        _ => 0,
    }
}
