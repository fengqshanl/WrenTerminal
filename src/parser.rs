//! 命令行解析器：分词、引号处理、变量展开、管道与重定向。

use crate::env::Env;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Word(String),
    Pipe,
    RedirOut,
    RedirAppend,
    RedirIn,
    /// `2>` / `2>>`：标准错误重定向。
    RedirErr,
    RedirErrAppend,
    /// `&>` / `&>>`：标准输出和标准错误一起重定向。
    RedirBoth,
    RedirBothAppend,
    /// `2>&1`：把标准错误合并到标准输出。
    MergeErr,
    AndIf,
    OrIf,
    Semi,
}

/// 引号内出现的通配符会被加上这个前缀，避免被当成 glob 展开。
/// 执行阶段会把它去掉。
pub const GLOB_ESCAPE: char = '\u{1}';

/// 给已转义的字符串里的通配符加保护标记。
pub fn protect_globs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '*' | '?' | '[' | ']') {
            out.push(GLOB_ESCAPE);
        }
        out.push(c);
    }
    out
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
    /// `2>` / `2>>` 的目标文件。
    pub stderr: Option<String>,
    pub stderr_append: bool,
    /// `2>&1`。
    pub merge_stderr: bool,
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
            // `2>` / `2>>` / `2>&1`：fd 数字后面紧跟 > 时按错误流处理。
            '1' | '2' if i + 1 < chars.len() && chars[i + 1] == '>' => {
                let fd = c;
                if i + 3 < chars.len() && chars[i + 2] == '&' && chars[i + 3] == '1' {
                    if fd == '2' {
                        tokens.push(Token::MergeErr);
                    }
                    i += 4;
                } else if i + 2 < chars.len() && chars[i + 2] == '>' {
                    tokens.push(if fd == '2' {
                        Token::RedirErrAppend
                    } else {
                        Token::RedirAppend
                    });
                    i += 3;
                } else {
                    tokens.push(if fd == '2' {
                        Token::RedirErr
                    } else {
                        Token::RedirOut
                    });
                    i += 2;
                }
            }
            // `&>` / `&>>`：标准输出和标准错误一起重定向。
            '&' if i + 1 < chars.len() && chars[i + 1] == '>' => {
                if i + 2 < chars.len() && chars[i + 2] == '>' {
                    tokens.push(Token::RedirBothAppend);
                    i += 3;
                } else {
                    tokens.push(Token::RedirBoth);
                    i += 2;
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
                // 单引号内的通配符按字面量处理，不做 glob 展开。
                tokens.push(Token::Word(protect_globs(&s)));
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
                tokens.push(Token::Word(protect_globs(&env.expand(&s))));
            }
            '\\' => {
                i += 1;
                if i < chars.len() {
                    let escaped = chars[i].to_string();
                    tokens.push(Token::Word(protect_globs(&env.expand(&escaped))));
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
        let has_redirect = redirect.input.is_some()
            || redirect.output.is_some()
            || redirect.stderr.is_some()
            || redirect.merge_stderr;
        if argv.is_empty() && !has_redirect {
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
                    match kind {
                        "input" => redirect.input = Some(w.clone()),
                        "stderr" => redirect.stderr = Some(w.clone()),
                        "both" => {
                            redirect.output = Some(w.clone());
                            redirect.stderr = Some(w.clone());
                        }
                        _ => redirect.output = Some(w.clone()),
                    }
                    continue;
                }
                _ => {
                    return Err(format!(
                        "{} 后面需要一个文件名",
                        match kind {
                            "input" => "<",
                            "stderr" => "2>",
                            "both" => "&>",
                            _ => ">",
                        }
                    ))
                }
            }
        }

        match tok {
            Token::Word(w) => argv.push(w.clone()),
            Token::RedirIn => expect_word = Some("input"),
            Token::RedirOut | Token::RedirAppend => {
                redirect.append = matches!(tok, Token::RedirAppend);
                expect_word = Some("output");
            }
            Token::RedirErr | Token::RedirErrAppend => {
                redirect.stderr_append = matches!(tok, Token::RedirErrAppend);
                expect_word = Some("stderr");
            }
            Token::RedirBoth | Token::RedirBothAppend => {
                let append = matches!(tok, Token::RedirBothAppend);
                redirect.append = append;
                redirect.stderr_append = append;
                expect_word = Some("both");
            }
            Token::MergeErr => redirect.merge_stderr = true,
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

    #[test]
    fn stderr_redirection() {
        let line = parse("cmd 2> err.log", &test_env()).unwrap();
        let r = &line.pipelines[0].1.stages[0].redirect;
        assert_eq!(r.stderr.as_deref(), Some("err.log"));
        assert!(!r.stderr_append);
        assert!(r.output.is_none());

        let line = parse("cmd 2>> err.log", &test_env()).unwrap();
        let r = &line.pipelines[0].1.stages[0].redirect;
        assert!(r.stderr_append);

        let line = parse("cmd 2>&1", &test_env()).unwrap();
        assert!(line.pipelines[0].1.stages[0].redirect.merge_stderr);

        let line = parse("cmd &> all.log", &test_env()).unwrap();
        let r = &line.pipelines[0].1.stages[0].redirect;
        assert_eq!(r.output.as_deref(), Some("all.log"));
        assert_eq!(r.stderr.as_deref(), Some("all.log"));
    }

    #[test]
    fn quoted_globs_are_protected() {
        let line = parse(r#"ls "*.rs" *.toml"#, &test_env()).unwrap();
        let argv = &line.pipelines[0].1.stages[0].argv;
        assert!(argv[1].contains(GLOB_ESCAPE));
        assert!(!argv[2].contains(GLOB_ESCAPE));
    }
}
