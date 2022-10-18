mod ast;
mod eval;
mod io;
mod line_editor;
mod terminal_size;

fn main() {
    terminal_size::init();

    let mut line_editor = line_editor::LineEditor::new();
    let mut shell = eval::Shell::new();
    let mut last_status = 0;

    loop {
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

        let job_status = match shell.jobs() {
            0 => "".to_owned(),
            1 => "*".to_owned(),
            num => format!("*{num}"),
        };

        let prompt_prefix = format!("[{}{}] {}", status, extra_status, job_status);

        use line_editor::EditError;
        let line = match line_editor.read_line(prompt_prefix) {
            Ok(line) => {
                println!();
                line
            }
            Err(EditError::Aborted) => {
                println!();
                continue;
            }
            Err(EditError::Exitted) => {
                if shell.jobs() == 0 {
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

        match ast::parser::toplevel(line) {
            Ok(program) => {
                last_status = shell.eval(&program);
            }
            Err(_err) => {
                eprintln!("Syntax Error");
                last_status = 127;
            }
        }
    }
}
