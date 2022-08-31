mod ast;

fn main() {
    use std::io::{stdin, stdout, Write as _};

    loop {
        print!("> ");
        stdout().flush().unwrap();

        let mut buf = String::new();
        stdin().read_line(&mut buf).unwrap();
        if buf.is_empty() {
            break;
        }

        let _ = dbg!(ast::parser::toplevel(&buf));
    }
}
