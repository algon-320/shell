pub type Program = List;

#[derive(Debug, PartialEq)]
pub struct List {
    pub first: Pipeline,
    pub following: Vec<(Condition, Pipeline)>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Condition {
    Always,
    IfSuccess,
    IfError,
}

#[derive(Debug, PartialEq)]
pub enum Pipeline {
    Single(Command),
    Connected {
        pipe: Pipe,
        lhs: Box<Pipeline>,
        rhs: Box<Pipeline>,
    },
}
#[derive(Debug, PartialEq, Eq)]
pub enum Pipe {
    Stdout,
    Stderr,
    Both,
}

#[derive(Debug, PartialEq)]
pub enum Command {
    Simple(Vec<Arguments>),
    SubShell(Box<List>),
}

#[derive(Debug, PartialEq)]
pub enum Arguments {
    Arg(Str),
    AtExpansion(Str),
}

pub type Str = Vec<StrPart>;

#[derive(Debug, PartialEq)]
pub enum StrPart {
    Chars(String),
    Expansion(Expansion),
}

#[derive(Debug, PartialEq)]
pub enum Expansion {
    SubstStdout(Box<List>),
    SubstStderr(Box<List>),
    SubstBoth(Box<List>),
    SubstPipeName(Box<List>),
    SubstStatus(Box<List>),
    Variable { name: String },
}

peg::parser! {
    pub grammar parser() for str {
        pub rule toplevel() -> Box<Program> = list()


        pub rule list() -> Box<List>
        = first:pipeline() following:(list_followings()*)
        { Box::new(List { first, following }) }

        rule list_followings() -> (Condition, Pipeline)
        = ";"  p:pipeline() { (Condition::Always, p) }
        / "&&" p:pipeline() { (Condition::IfSuccess, p) }
        / "||" p:pipeline() { (Condition::IfError, p) }

        pub rule pipeline() -> Pipeline
        = "{" lhs:pipeline() "}" pipe:pipe() rhs:pipeline() {
            let lhs = Box::new(lhs);
            let rhs = Box::new(rhs);
            Pipeline::Connected { pipe, lhs, rhs }
        }
        / "{" p:pipeline() "}" {
            p
        }
        / cmd:command() pipe:pipe() rhs:pipeline() {
            let lhs = Box::new(Pipeline::Single(cmd));
            let rhs = Box::new(rhs);
            Pipeline::Connected { pipe, lhs, rhs }
        }
        / cmd:command() {
            Pipeline::Single(cmd)
        }

        rule pipe() -> Pipe
        = ws()* "|&" ws()* { Pipe::Both }
        / ws()* "|!" ws()* { Pipe::Stderr }
        / ws()* "|"  ws()* { Pipe::Stdout }


        pub rule command() -> Command
        = ws()* sub:subshell() ws()* { Command::SubShell(sub) }
        / cmd:simple_command()       { Command::Simple(cmd) }

        rule subshell() -> Box<List> = "(" list:list() ")" { list }

        rule simple_command() -> Vec<Arguments>
        = args:(arguments()+) { args }
        rule arguments() -> Arguments
        = ws()* "@" s:string() ws()* { Arguments::AtExpansion(s) }
        / ws()*     s:string() ws()* { Arguments::Arg(s) }

        rule ident() -> String
        = s:$(['a'..='z' | 'A'..='Z' | '_']['a'..='z' | 'A'..='Z' | '_' | '0'..='9']*)
        { s.to_string() }


        pub rule string() -> Str
        = text:single_quoted()  { vec![StrPart::Chars(text)] }
        / parts:double_quoted() { parts }
        / parts:raw()           { parts }

        rule single_quoted() -> String
        = "'" chars:(single_quoted_char()*) "'" { chars.into_iter().collect() }

        rule single_quoted_char() -> char
        = r#"\'"# { '\'' }
        / r#"\\"# { '\\' }
        / c:[^ '\''] { c }

        rule double_quoted() -> Vec<StrPart>
        = "\"" parts:(double_quoted_str_part()*) "\"" { parts }

        rule double_quoted_str_part() -> StrPart
        = e:expansion() { StrPart::Expansion(e) }
        / c:(double_quoted_char()+) { StrPart::Chars(c.into_iter().collect()) }

        rule double_quoted_char() -> char
        = r#"\""# { '"' }
        / r#"\\"# { '\\' }
        / r#"\$"# { '$' }
        / c:[^ '"' | '$'] { c }

        rule raw() -> Vec<StrPart>
        = parts:(raw_str_part()+) { parts }

        rule raw_str_part() -> StrPart
        = e:expansion() { StrPart::Expansion(e) }
        / c:(raw_char()+) { StrPart::Chars(c.into_iter().collect()) }

        rule raw_char() -> char
        = ['\\'] c:[  '\\'|' '|'\t'|'\n'|'@'|';'|'&'|'|'|'$'|'('|')'|'['|']'|'\''|'\"'|'='|'?'|'{'|'}'|'*'] { c }
        /        c:[^ '\\'|' '|'\t'|'\n'|'@'|';'|'&'|'|'|'$'|'('|')'|'['|']'|'\''|'\"'|'='|'?'|'{'|'}'] { c }
        / !"=(" ['='] { '=' }
        / !"?(" ['?'] { '?' }


        pub rule expansion() -> Expansion
        = "$&" list:subshell() { Expansion::SubstBoth(list) }
        / "$!" list:subshell() { Expansion::SubstStderr(list) }
        / "$"  list:subshell() { Expansion::SubstStdout(list) }
        / "="  list:subshell() { Expansion::SubstPipeName(list) }
        / "?"  list:subshell() { Expansion::SubstStatus(list) }
        / name:variable()      { Expansion::Variable { name } }

        rule variable() -> String
        = "${" name:ident() "}" { name.to_string() }
        / "$"  name:ident()     { name.to_string() }

        rule ws() = [' '|'\t'|'\n'|'\r']
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple() {
        let input = "foo";
        let expected = Command::Simple(vec![Arguments::Arg(vec![StrPart::Chars("foo".into())])]);
        assert_eq!(parser::command(input), Ok(expected));

        let input = "  foo  ";
        let expected = Command::Simple(vec![Arguments::Arg(vec![StrPart::Chars("foo".into())])]);
        assert_eq!(parser::command(input), Ok(expected));

        let input = "foo bar";
        let expected = Command::Simple(vec![
            Arguments::Arg(vec![StrPart::Chars("foo".into())]),
            Arguments::Arg(vec![StrPart::Chars("bar".into())]),
        ]);
        assert_eq!(parser::command(input), Ok(expected));

        let input = "foo @xxx";
        let expected = Command::Simple(vec![
            Arguments::Arg(vec![StrPart::Chars("foo".into())]),
            Arguments::AtExpansion(vec![StrPart::Chars("xxx".into())]),
        ]);
        assert_eq!(parser::command(input), Ok(expected));

        let input = "foo arg1 @args";
        let expected = Command::Simple(vec![
            Arguments::Arg(vec![StrPart::Chars("foo".into())]),
            Arguments::Arg(vec![StrPart::Chars("arg1".into())]),
            Arguments::AtExpansion(vec![StrPart::Chars("args".into())]),
        ]);
        assert_eq!(parser::command(input), Ok(expected));
    }

    #[test]
    fn parse_subshell() {
        let input = "(foo)";
        let expected = Command::SubShell(
            List {
                first: Pipeline::Single(Command::Simple(vec![Arguments::Arg(vec![
                    StrPart::Chars("foo".into()),
                ])])),
                following: Vec::new(),
            }
            .into(),
        );
        assert_eq!(parser::command(input), Ok(expected));

        let input = "(foo bar)";
        let expected = Command::SubShell(
            List {
                first: Pipeline::Single(Command::Simple(vec![
                    Arguments::Arg(vec![StrPart::Chars("foo".into())]),
                    Arguments::Arg(vec![StrPart::Chars("bar".into())]),
                ])),
                following: Vec::new(),
            }
            .into(),
        );
        assert_eq!(parser::command(input), Ok(expected));
    }

    #[test]
    fn parse_pipeline() {
        let input = "foo | bar";
        let expected = Pipeline::Connected {
            pipe: Pipe::Stdout,
            lhs: Pipeline::Single(Command::Simple(vec![Arguments::Arg(vec![StrPart::Chars(
                "foo".into(),
            )])]))
            .into(),
            rhs: Pipeline::Single(Command::Simple(vec![Arguments::Arg(vec![StrPart::Chars(
                "bar".into(),
            )])]))
            .into(),
        };
        assert_eq!(parser::pipeline(input), Ok(expected));

        let input = "foo |! bar";
        let expected = Pipeline::Connected {
            pipe: Pipe::Stderr,
            lhs: Pipeline::Single(Command::Simple(vec![Arguments::Arg(vec![StrPart::Chars(
                "foo".into(),
            )])]))
            .into(),
            rhs: Pipeline::Single(Command::Simple(vec![Arguments::Arg(vec![StrPart::Chars(
                "bar".into(),
            )])]))
            .into(),
        };
        assert_eq!(parser::pipeline(input), Ok(expected));

        let input = "foo |& bar";
        let expected = Pipeline::Connected {
            pipe: Pipe::Both,
            lhs: Pipeline::Single(Command::Simple(vec![Arguments::Arg(vec![StrPart::Chars(
                "foo".into(),
            )])]))
            .into(),
            rhs: Pipeline::Single(Command::Simple(vec![Arguments::Arg(vec![StrPart::Chars(
                "bar".into(),
            )])]))
            .into(),
        };
        assert_eq!(parser::pipeline(input), Ok(expected));
    }

    #[test]
    fn parse_list() {
        let input = "foo ; bar";
        let expected = Box::new(List {
            first: Pipeline::Single(Command::Simple(vec![Arguments::Arg(vec![StrPart::Chars(
                "foo".into(),
            )])])),
            following: vec![(
                Condition::Always,
                Pipeline::Single(Command::Simple(vec![Arguments::Arg(vec![StrPart::Chars(
                    "bar".into(),
                )])])),
            )],
        });
        assert_eq!(parser::list(input), Ok(expected));

        let input = "foo && bar";
        let expected = Box::new(List {
            first: Pipeline::Single(Command::Simple(vec![Arguments::Arg(vec![StrPart::Chars(
                "foo".into(),
            )])])),
            following: vec![(
                Condition::IfSuccess,
                Pipeline::Single(Command::Simple(vec![Arguments::Arg(vec![StrPart::Chars(
                    "bar".into(),
                )])])),
            )],
        });
        assert_eq!(parser::list(input), Ok(expected));

        let input = "foo || bar";
        let expected = Box::new(List {
            first: Pipeline::Single(Command::Simple(vec![Arguments::Arg(vec![StrPart::Chars(
                "foo".into(),
            )])])),
            following: vec![(
                Condition::IfError,
                Pipeline::Single(Command::Simple(vec![Arguments::Arg(vec![StrPart::Chars(
                    "bar".into(),
                )])])),
            )],
        });
        assert_eq!(parser::list(input), Ok(expected));
    }

    #[test]
    fn parse_str_single_quote() {
        let input = r#"'foo bar'"#;
        let expected = vec![StrPart::Chars("foo bar".into())];
        assert_eq!(parser::string(input), Ok(expected));

        let input = r#"'\''"#;
        let expected = vec![StrPart::Chars("'".into())];
        assert_eq!(parser::string(input), Ok(expected));

        let input = r#"'\\'"#;
        let expected = vec![StrPart::Chars("\\".into())];
        assert_eq!(parser::string(input), Ok(expected));

        let input = r#"'"!@#$%^&*()_+-='"#;
        let expected = vec![StrPart::Chars("\"!@#$%^&*()_+-=".into())];
        assert_eq!(parser::string(input), Ok(expected));
    }

    #[test]
    fn parse_str_double_quote() {
        let input = r#""foo bar""#;
        let expected = vec![StrPart::Chars(r#"foo bar"#.into())];
        assert_eq!(parser::string(input), Ok(expected));

        let input = r#""'""#;
        let expected = vec![StrPart::Chars(r#"'"#.into())];
        assert_eq!(parser::string(input), Ok(expected));

        let input = r#""\"""#;
        let expected = vec![StrPart::Chars(r#"""#.into())];
        assert_eq!(parser::string(input), Ok(expected));

        let input = r#""\\""#;
        let expected = vec![StrPart::Chars(r#"\"#.into())];
        assert_eq!(parser::string(input), Ok(expected));

        let input = r#""\$""#;
        let expected = vec![StrPart::Chars(r#"$"#.into())];
        assert_eq!(parser::string(input), Ok(expected));

        let input = r#""!@#%^&*()_+-=""#;
        let expected = vec![StrPart::Chars(r#"!@#%^&*()_+-="#.into())];
        assert_eq!(parser::string(input), Ok(expected));
    }

    #[test]
    fn parse_str_raw() {
        let input = r#"foo"#;
        let expected = vec![StrPart::Chars(r#"foo"#.into())];
        assert_eq!(parser::string(input), Ok(expected));

        let input = r#"foo\ bar"#;
        let expected = vec![StrPart::Chars(r#"foo bar"#.into())];
        assert_eq!(parser::string(input), Ok(expected));

        let input = r#"\'"#;
        let expected = vec![StrPart::Chars(r#"'"#.into())];
        assert_eq!(parser::string(input), Ok(expected));

        let input = r#"\""#;
        let expected = vec![StrPart::Chars(r#"""#.into())];
        assert_eq!(parser::string(input), Ok(expected));

        let input = r#"\\"#;
        let expected = vec![StrPart::Chars(r#"\"#.into())];
        assert_eq!(parser::string(input), Ok(expected));

        let input = r#"\$"#;
        let expected = vec![StrPart::Chars(r#"$"#.into())];
        assert_eq!(parser::string(input), Ok(expected));

        let input = r#"="#;
        let expected = vec![StrPart::Chars(r#"="#.into())];
        assert_eq!(parser::string(input), Ok(expected));

        let input = r#"\=\(\)"#;
        let expected = vec![StrPart::Chars(r#"=()"#.into())];
        assert_eq!(parser::string(input), Ok(expected));
    }

    #[test]
    fn parse_variable() {
        let input = r#"$xxx"#;
        let expected = Expansion::Variable { name: "xxx".into() };
        assert_eq!(parser::expansion(input), Ok(expected));
    }

    #[test]
    fn parse_subst() {
        let input = r#"$(foo)"#;
        let expected = Expansion::SubstStdout(
            List {
                first: Pipeline::Single(Command::Simple(vec![Arguments::Arg(vec![
                    StrPart::Chars("foo".into()),
                ])])),
                following: Vec::new(),
            }
            .into(),
        );
        assert_eq!(parser::expansion(input), Ok(expected));

        let input = r#"$!(foo)"#;
        let expected = Expansion::SubstStderr(
            List {
                first: Pipeline::Single(Command::Simple(vec![Arguments::Arg(vec![
                    StrPart::Chars("foo".into()),
                ])])),
                following: Vec::new(),
            }
            .into(),
        );
        assert_eq!(parser::expansion(input), Ok(expected));

        let input = r#"$&(foo)"#;
        let expected = Expansion::SubstBoth(
            List {
                first: Pipeline::Single(Command::Simple(vec![Arguments::Arg(vec![
                    StrPart::Chars("foo".into()),
                ])])),
                following: Vec::new(),
            }
            .into(),
        );
        assert_eq!(parser::expansion(input), Ok(expected));

        let input = r#"=(foo)"#;
        let expected = Expansion::SubstPipeName(
            List {
                first: Pipeline::Single(Command::Simple(vec![Arguments::Arg(vec![
                    StrPart::Chars("foo".into()),
                ])])),
                following: Vec::new(),
            }
            .into(),
        );
        assert_eq!(parser::expansion(input), Ok(expected));

        let input = r#"?(foo)"#;
        let expected = Expansion::SubstStatus(
            List {
                first: Pipeline::Single(Command::Simple(vec![Arguments::Arg(vec![
                    StrPart::Chars("foo".into()),
                ])])),
                following: Vec::new(),
            }
            .into(),
        );
        assert_eq!(parser::expansion(input), Ok(expected));
    }

    #[test]
    fn parse_toplevel() {
        let input = r#"(foo)"#;

        let expected = Box::new(List {
            first: Pipeline::Single(Command::SubShell(
                List {
                    first: Pipeline::Single(Command::Simple(vec![Arguments::Arg(vec![
                        StrPart::Chars("foo".into()),
                    ])])),
                    following: Vec::new(),
                }
                .into(),
            )),
            following: Vec::new(),
        });
        assert_eq!(parser::toplevel(input), Ok(expected));

        let input = r#"a "xxx_$(b |!> err)_yyy" \$zzz $zzz ; (baz)"#;

        let expected = Box::new(List {
            first: Pipeline::Single(Command::Simple(vec![
                Arguments::Arg(vec![StrPart::Chars("a".into())]),
                Arguments::Arg(vec![
                    StrPart::Chars("xxx_".into()),
                    StrPart::Expansion(Expansion::SubstStdout(
                        List {
                            first: Pipeline::Connected {
                                pipe: Pipe::Stderr,
                                lhs: Pipeline::Single(Command::Simple(vec![Arguments::Arg(vec![
                                    StrPart::Chars("b".into()),
                                ])]))
                                .into(),
                                rhs: Pipeline::Single(Command::Simple(vec![
                                    Arguments::Arg(vec![StrPart::Chars(">".into())]),
                                    Arguments::Arg(vec![StrPart::Chars("err".into())]),
                                ]))
                                .into(),
                            },
                            following: Vec::new(),
                        }
                        .into(),
                    )),
                    StrPart::Chars("_yyy".into()),
                ]),
                Arguments::Arg(vec![StrPart::Chars("$zzz".into())]),
                Arguments::Arg(vec![StrPart::Expansion(Expansion::Variable {
                    name: "zzz".into(),
                })]),
            ])),
            following: vec![(
                Condition::Always,
                Pipeline::Single(Command::SubShell(
                    List {
                        first: Pipeline::Single(Command::Simple(vec![Arguments::Arg(vec![
                            StrPart::Chars("baz".into()),
                        ])])),
                        following: Vec::new(),
                    }
                    .into(),
                )),
            )],
        });
        assert_eq!(parser::toplevel(input), Ok(expected));
    }
}
