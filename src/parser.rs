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
            '\'' | '"' | '\\' | _ => {
                // 普通词：支持引号拼接（`a'b c'`、`"x"y`）与内联变量展开。
                let (word, next) = scan_word(&chars, i, env)?;
                tokens.push(Token::Word(word));
                i = next;
            }
        }
    }
    Ok(tokens)
}

/// 扫描一个“词”，直到遇到未加引号的空白或控制符。
///
/// 与 bash 一致：
/// - 单引号内不做变量展开和 glob
/// - 双引号内做变量展开，但不做 glob
/// - 引号可以与普通字符拼接成一个词
fn scan_word(chars: &[char], mut i: usize, env: &Env) -> Result<(String, usize), String> {
    let mut s = String::new();
    // 记录 ~ 是否位于词首，用于主目录展开。
    let at_start = true;
    let mut first = true;

    while i < chars.len() {
        let ch = chars[i];
        if ch.is_whitespace() || matches!(ch, '|' | '>' | '<' | ';' | '&') {
            break;
        }
        match ch {
            '\\' => {
                if i + 1 < chars.len() {
                    s.push(GLOB_ESCAPE);
                    s.push(chars[i + 1]);
                    i += 2;
                } else {
                    s.push('\\');
                    i += 1;
                }
            }
            '\'' => {
                i += 1;
                let mut part = String::new();
                while i < chars.len() && chars[i] != '\'' {
                    part.push(chars[i]);
                    i += 1;
                }
                if i >= chars.len() {
                    return Err("单引号未闭合".into());
                }
                i += 1;
                s.push_str(&protect_globs(&part));
            }
            '"' => {
                i += 1;
                let mut part = String::new();
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        let n = chars[i + 1];
                        if matches!(n, '"' | '\\' | '$' | '`') {
                            part.push(n);
                            i += 2;
                            continue;
                        }
                    }
                    part.push(chars[i]);
                    i += 1;
                }
                if i >= chars.len() {
                    return Err("双引号未闭合".into());
                }
                i += 1;
                s.push_str(&protect_globs(&env.expand(&part)));
            }
            '$' => {
                let (text, next) = scan_dollar(chars, i, env);
                s.push_str(&text);
                i = next;
            }
            '~' if first && at_start => {
                // 词首的 ~ 展开为主目录（~ 后必须跟 / 或结束）。
                let next_is_sep = i + 1 >= chars.len()
                    || chars[i + 1] == '/'
                    || chars[i + 1].is_whitespace()
                    || matches!(chars[i + 1], '|' | '>' | '<' | ';' | '&');
                if next_is_sep {
                    s.push_str(&env.home_string());
                    i += 1;
                } else {
                    s.push(ch);
                    i += 1;
                }
            }
            _ => {
                s.push(ch);
                i += 1;
            }
        }
        first = false;
    }
    Ok((s, i))
}

/// 展开 `$NAME`、`${NAME}`、`$?`、`$$`，返回 (文本, 新位置)。
fn scan_dollar(chars: &[char], i: usize, env: &Env) -> (String, usize) {
    if i + 1 >= chars.len() {
        return ("$".to_string(), i + 1);
    }
    match chars[i + 1] {
        '?' => ("\u{2}".to_string(), i + 2),
        '$' => (std::process::id().to_string(), i + 2),
        '{' => {
            if let Some(end) = chars[i + 2..].iter().position(|&c| c == '}') {
                let name: String = chars[i + 2..i + 2 + end].iter().collect();
                (env.get_or(&name, ""), i + 2 + end + 1)
            } else {
                ("$".to_string(), i + 1)
            }
        }
        c if c.is_ascii_alphabetic() || c == '_' => {
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let name: String = chars[i + 1..j].iter().collect();
            (env.get_or(&name, ""), j)
        }
        _ => ("$".to_string(), i + 1),
    }
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

    #[test]
    fn quotes_concatenate_into_one_word() {
        let line = parse(r#"alias ll='ls -l'"#, &test_env()).unwrap();
        let argv = &line.pipelines[0].1.stages[0].argv;
        assert_eq!(argv, &["alias", "ll=ls -l"]);

        let line = parse(r#"echo a"b c"d"#, &test_env()).unwrap();
        assert_eq!(line.pipelines[0].1.stages[0].argv[1], "ab cd");

        let line = parse(r#"export FOO="a b""#, &test_env()).unwrap();
        assert_eq!(line.pipelines[0].1.stages[0].argv[1], "FOO=a b");
    }

    #[test]
    fn tilde_and_variables_inside_words() {
        let line = parse("echo ~/x $FOO pre$FOO", &test_env()).unwrap();
        let argv = &line.pipelines[0].1.stages[0].argv;
        assert!(argv[1].ends_with("/x"));
        assert_eq!(argv[2], "bar");
        assert_eq!(argv[3], "prebar");
    }
}
