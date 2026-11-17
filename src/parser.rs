//! 命令行解析器：分词、引号处理、变量展开、管道与重定向。

use crate::env::Env;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Word(String),
    Pipe,
    RedirOut,
    RedirAppend,
    RedirIn,
    AndIf,
    OrIf,
    Semi,
}

/// 多条命令之间的连接方式。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Conn {
    Always, // ; 或第一条
    And,    // &&
    Or,     // ||
}

#[derive(Debug, Clone, Default)]
pub struct Redirect {
    pub input: Option<String>,
    pub output: Option<String>,
    pub append: bool,
}

#[derive(Debug, Clone)]
pub struct Stage {
    pub argv: Vec<String>,
    pub redirect: Redirect,
}

#[derive(Debug, Clone)]
pub struct Pipeline {
    pub stages: Vec<Stage>,
}

#[derive(Debug, Clone)]
pub struct ParsedLine {
    pub pipelines: Vec<(Conn, Pipeline)>,
}

/// 把输入行切分为 token。变量展开在此时完成（单引号内不展开）。
pub fn tokenize(input: &str, env: &Env) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }

        match c {
            '|' => {
                if i + 1 < chars.len() && chars[i + 1] == '|' {
                    tokens.push(Token::OrIf);
                    i += 2;
                } else {
                    tokens.push(Token::Pipe);
                    i += 1;
                }
            }
            '>' => {
                if i + 1 < chars.len() && chars[i + 1] == '>' {
                    tokens.push(Token::RedirAppend);
                    i += 2;
                } else {
                    tokens.push(Token::RedirOut);
                    i += 1;
                }
            }
            '<' => {
                tokens.push(Token::RedirIn);
                i += 1;
            }
            ';' => {
                tokens.push(Token::Semi);
                i += 1;
            }
            '&' => {
                if i + 1 < chars.len() && chars[i + 1] == '&' {
                    tokens.push(Token::AndIf);
                    i += 2;
                } else {
                    return Err("不支持后台任务 '&'（末尾单 &）".into());
                }
            }
            '\'' => {
                i += 1;
                let mut s = String::new();
                while i < chars.len() && chars[i] != '\'' {
                    s.push(chars[i]);
                    i += 1;
                }
                if i >= chars.len() {
                    return Err("单引号未闭合".into());
                }
                i += 1;
                tokens.push(Token::Word(s));
            }
            '"' => {
                i += 1;
                let mut s = String::new();
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        let n = chars[i + 1];
                        if matches!(n, '"' | '\\' | '$' | '`') {
                            s.push(n);
                            i += 2;
                            continue;
                        }
                    }
                    s.push(chars[i]);
                    i += 1;
                }
                if i >= chars.len() {
                    return Err("双引号未闭合".into());
                }
                i += 1;
                tokens.push(Token::Word(env.expand(&s)));
            }
            '\\' => {
                i += 1;
                if i < chars.len() {
                    tokens.push(Token::Word(env.expand(&chars[i].to_string())));
                    i += 1;
                } else {
                    return Err("转义符 '\\' 后缺少字符".into());
                }
            }
            _ => {
                let mut s = String::new();
                while i < chars.len() {
                    let ch = chars[i];
                    if ch == '\\' {
                        if i + 1 < chars.len() {
                            s.push(chars[i + 1]);
                            i += 2;
                            continue;
                        }
                        s.push('\\');
                        i += 1;
                        continue;
                    }
                    if ch.is_whitespace()
                        || matches!(ch, '|' | '>' | '<' | ';' | '&' | '\'' | '"')
                    {
                        break;
                    }
                    s.push(ch);
                    i += 1;
                }
                if !s.is_empty() {
                    tokens.push(Token::Word(env.expand(&s)));
                }
            }
        }
    }
    Ok(tokens)
}

/// 把 token 序列解析为可执行的命令列表。
pub fn parse(input: &str, env: &Env) -> Result<ParsedLine, String> {
    let tokens = tokenize(input, env)?;
    if tokens.is_empty() {
        return Ok(ParsedLine { pipelines: vec![] });
    }

    let mut pipelines: Vec<(Conn, Pipeline)> = Vec::new();
    let mut stages: Vec<Stage> = Vec::new();
    let mut argv: Vec<String> = Vec::new();
    let mut redirect = Redirect::default();
    let mut conn = Conn::Always;
    let mut expect_word: Option<&str> = None;

    let flush_stage = |stages: &mut Vec<Stage>, argv: &mut Vec<String>, redirect: &mut Redirect| -> Result<(), String> {
        if argv.is_empty() && redirect.input.is_none() && redirect.output.is_none() {
            return Ok(());
        }
        if argv.is_empty() {
            return Err("重定向缺少命令".into());
        }
        stages.push(Stage {
            argv: std::mem::take(argv),
            redirect: std::mem::take(redirect),
        });
        Ok(())
    };

    for tok in &tokens {
        if let Some(kind) = expect_word.take() {
            match tok {
                Token::Word(w) => {
                    if kind == "input" {
                        redirect.input = Some(w.clone());
                    } else {
                        redirect.output = Some(w.clone());
                    }
                    continue;
                }
                _ => return Err(format!("{} 后面需要一个文件名", if kind == "input" { "<" } else { ">" })),
            }
        }

        match tok {
            Token::Word(w) => argv.push(w.clone()),
            Token::RedirIn => expect_word = Some("input"),
            Token::RedirOut | Token::RedirAppend => {
                redirect.append = matches!(tok, Token::RedirAppend);
                expect_word = Some("output");
            }
            Token::Pipe => {
                flush_stage(&mut stages, &mut argv, &mut redirect)?;
                if stages.is_empty() {
                    return Err("管道 '|' 前缺少命令".into());
                }
            }
            Token::Semi | Token::AndIf | Token::OrIf => {
                flush_stage(&mut stages, &mut argv, &mut redirect)?;
                if stages.is_empty() {
                    return Err("连接符前缺少命令".into());
                }
                pipelines.push((
                    conn,
                    Pipeline {
                        stages: std::mem::take(&mut stages),
                    },
                ));
                conn = match tok {
                    Token::AndIf => Conn::And,
                    Token::OrIf => Conn::Or,
                    _ => Conn::Always,
                };
            }
        }
    }

    if expect_word.is_some() {
        return Err("重定向缺少文件名".into());
    }
    flush_stage(&mut stages, &mut argv, &mut redirect)?;
    if !stages.is_empty() {
        pipelines.push((conn, Pipeline { stages }));
    }

    Ok(ParsedLine { pipelines })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_env() -> Env {
        let mut e = Env::new("test".into());
        e.set("FOO", "bar");
        e
    }

    #[test]
    fn basic_pipeline() {
        let line = parse("cat a.txt | grep -n foo > out.txt", &test_env()).unwrap();
        assert_eq!(line.pipelines.len(), 1);
        let stages = &line.pipelines[0].1.stages;
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0].argv, vec!["cat", "a.txt"]);
        assert_eq!(stages[1].argv, vec!["grep", "-n", "foo"]);
        assert_eq!(stages[1].redirect.output.as_deref(), Some("out.txt"));
    }

    #[test]
    fn variable_and_quotes() {
        let line = parse("echo \"hello $FOO\" '$FOO'", &test_env()).unwrap();
        let argv = &line.pipelines[0].1.stages[0].argv;
        assert_eq!(argv, &["echo", "hello bar", "$FOO"]);
    }

    #[test]
    fn connectors() {
        let line = parse("a && b ; c || d", &test_env()).unwrap();
        assert_eq!(line.pipelines.len(), 4);
        assert_eq!(line.pipelines[1].0, Conn::And);
        assert_eq!(line.pipelines[2].0, Conn::Always);
        assert_eq!(line.pipelines[3].0, Conn::Or);
    }
}
