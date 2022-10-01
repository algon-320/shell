mod ast;
mod eval;
mod io;

fn main() {
    use std::io::{stdin, stdout, Write as _};

    let mut shell = eval::Shell::new();

    let mut last_status = 0;
    loop {
        let job_status = if shell.jobs() > 0 {
            "*".repeat(shell.jobs())
        } else {
            "".to_owned()
        };
        print!("[{:3}] {}>> ", last_status, job_status);
        stdout().flush().unwrap();

        let mut buf = String::new();
        stdin().read_line(&mut buf).unwrap();

        if buf.is_empty() {
            break;
        }

        if buf.as_str() == "\n" {
            continue;
        }

        match ast::parser::toplevel(buf.trim_end()) {
            Ok(program) => {
                last_status = shell.eval(&program);
            }
            Err(err) => {
                dbg!(err);
            }
        }
    }
}
