mod completion;
mod core;
mod line_editor;
mod terminal_size;
mod utils;

fn main() {
    terminal_size::install_sigwinch_handler();

    let mut line_editor = line_editor::LineEditor::new();
    let mut shell = core::Shell::new();
    let mut last_status = eval_startup(&mut shell).unwrap_or(0);

    loop {
        terminal_size::update();
        shell.update_variables();

        line_editor
            .command_completion
            .update_commands(shell.list_commands());

        let prompt_prefix = {
            let status_style = if last_status == 0 {
                // successful
                "\x1b[32m"
            } else if last_status < 128 {
                // error
                "\x1b[31m"
            } else {
                // signaled
                "\x1b[33m"
            };

            let cwd_style = "\x1b[1;35m";
            let cwd = match std::env::current_dir() {
                Err(_) => "unknown".to_owned(),
                Ok(cwd) => std::env::var("HOME")
                    .ok()
                    .and_then(|home| cwd.strip_prefix(&home).ok())
                    .map(|p| format!("~/{}", p.display()))
                    .unwrap_or_else(|| cwd.display().to_string()),
            };

            let job_indicator = match shell.jobs() {
                0 => "".to_owned(),
                1 => "*".to_owned(),
                num => format!("*{num}"),
            };

            format!(
                "(\x1b[m)[({status_style}){:3}(\x1b[m)] ({cwd_style}){}(\x1b[m) {}",
                last_status, cwd, job_indicator
            )
        };

        match line_editor.read_line(prompt_prefix) {
            Ok(line) => {
                let line = line.trim();
                if !line.is_empty() {
                    last_status = shell.eval(line);
                }
            }

            Err(line_editor::EditError::Aborted) => {}

            Err(line_editor::EditError::Exitted) => {
                if shell.jobs() == 0 {
                    break;
                } else {
                    println!("You have suspended jobs.");
                }
            }
        }
    }
}

fn eval_startup(shell: &mut core::Shell) -> Option<i32> {
    use std::io::{BufRead as _, BufReader};

    let app_dir = application_dir()?;
    let mut file_path = app_dir;
    file_path.push("startup");

    let file = match std::fs::File::open(&file_path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        _ => return Some(1),
    };

    let mut status = 0;
    for line in BufReader::new(file).lines().filter_map(|r| r.ok()) {
        let line = line.trim();
        if !line.is_empty() {
            status = shell.eval(line);
        }
    }
    Some(status)
}

// TODO: consider being XDG complient
fn application_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut p = std::path::PathBuf::from(home);
    p.push(".myshell");
    Some(p)
}
