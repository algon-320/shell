mod completion;
mod core;
mod line_editor;
mod terminal_size;
mod utils;

fn main() {
    terminal_size::init();

    let mut line_editor = line_editor::LineEditor::new();
    let mut shell = core::Shell::new();
    let mut last_status = eval_starup(&mut shell);

    loop {
        terminal_size::update();
        shell.update_variables();

        line_editor
            .command_completion
            .update_commands(shell.list_commands());

        let status = if last_status == 0 {
            // successful
            format!("(\x1b[32m){:3}(\x1b[m)", last_status)
        } else if last_status < 128 {
            // error
            format!("(\x1b[31m){:3}(\x1b[m)", last_status)
        } else {
            // signaled
            format!("(\x1b[33m){:3}(\x1b[m)", last_status)
        };

        let cwd = format!(
            "(\x1b[1;35m){}(\x1b[m)",
            std::env::current_dir()
                .ok()
                .map(|p| {
                    if let Some(path_after_home) = std::env::var("HOME")
                        .ok()
                        .and_then(|home| p.strip_prefix(home).ok())
                    {
                        format!("~/{}", path_after_home.display())
                    } else {
                        p.display().to_string()
                    }
                })
                .unwrap_or_else(|| "unknown".to_owned())
        );

        let job_status = match shell.jobs() {
            0 => "".to_owned(),
            1 => "*".to_owned(),
            num => format!("*{num}"),
        };

        let prompt_prefix = format!("(\x1b[m)[{}] {} {}", status, cwd, job_status);

        use line_editor::EditError;
        let line = match line_editor.read_line(prompt_prefix) {
            Ok(line) => line,

            Err(EditError::Aborted) => {
                continue;
            }

            Err(EditError::Exitted) => {
                if shell.jobs() == 0 {
                    break;
                } else {
                    println!("You have suspended jobs.");
                    continue;
                }
            }
        };

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        last_status = shell.eval(line);
    }
}

fn eval_starup(shell: &mut core::Shell) -> i32 {
    use std::io::{BufRead as _, BufReader};

    if let Some(app_dir) = application_dir() {
        let mut file_path = app_dir;
        file_path.push("startup");

        let file = match std::fs::File::open(&file_path) {
            Ok(file) => file,
            _ => return 1,
        };

        let mut status = 0;
        for line in BufReader::new(file).lines().filter_map(|r| r.ok()) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            status = shell.eval(line);
        }
        status
    } else {
        0
    }
}

// TODO: consider being XDG complient
fn application_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut p = std::path::PathBuf::from(home);
    p.push(".myshell");
    Some(p)
}
