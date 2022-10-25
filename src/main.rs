mod completion;
mod core;
mod line_editor;
mod terminal_size;

fn main() {
    terminal_size::init();

    let mut line_editor = line_editor::LineEditor::new();
    let mut shell = core::Shell::new();
    let mut last_status = 0;

    loop {
        terminal_size::update();

        line_editor
            .command_completion
            .update_commands(shell.list_commands());

        let status = if last_status == 0 {
            format!("(\x1b[32m){:3}(\x1b[m)", last_status)
        } else {
            format!("(\x1b[31m){:3}(\x1b[m)", last_status)
        };

        let extra_status = if last_status >= 128 {
            format!(
                ":(\x1b[33m){}(\x1b[m)",
                nix::sys::signal::Signal::try_from(last_status - 128).unwrap()
            )
        } else {
            "".to_owned()
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

        let prompt_prefix = format!(
            "(\x1b[m)[{}{}] {} {}",
            status, extra_status, cwd, job_status
        );

        use line_editor::EditError;
        let line = match line_editor.read_line(prompt_prefix) {
            Ok(line) => line,
            Err(EditError::Aborted) => {
                println!();
                continue;
            }
            Err(EditError::Exitted) => {
                if shell.jobs() == 0 {
                    println!("exit");
                    break;
                } else {
                    println!();
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
