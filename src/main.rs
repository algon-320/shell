mod ast;
mod eval;
mod io;
mod line_editor;

fn main() {
    use std::io::{stdout, Write as _};

    let mut line_editor = line_editor::LineEditor::new();
    let mut shell = eval::Shell::new();
    let mut last_status = 0;

    loop {
        let status = if last_status == 0 {
            format!("\x1b[32m{:3}\x1b[m", last_status)
        } else {
            format!("\x1b[31m{:3}\x1b[m", last_status)
        };

        let job_status = if shell.jobs() > 0 {
            "*".repeat(shell.jobs())
        } else {
            "".to_owned()
        };

        let extra_status = if last_status >= 128 {
            format!(
                ":\x1b[33m{}\x1b[m",
                nix::sys::signal::Signal::try_from(last_status - 128).unwrap()
            )
        } else {
            "".to_owned()
        };

        print!("[{}{}] {}% ", status, extra_status, job_status);
        stdout().flush().unwrap();

        use line_editor::EditError;
        let line = match line_editor.read_line() {
            Ok(line) => {
                println!();
                line
            }
            Err(EditError::Aborted) => {
                println!();
                continue;
            }
            Err(EditError::Exitted) => {
                break;
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
