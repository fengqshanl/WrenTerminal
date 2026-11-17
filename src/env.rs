//! 环境变量：维护一份 Linux 风格的环境视图，并与 Windows 进程环境双向同步。

use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;

/// 把用户名中的中文字符转成拼音（仅内置常见字，够用作提示符显示）。
/// 若输入本身已是 ASCII 则原样返回；若含无法映射的字符则返回 None。
fn to_ascii_name(name: &str) -> Option<String> {
    if name.is_ascii() && !name.is_empty() {
        return Some(name.to_string());
    }
    // 常见汉字 -> 拼音映射（覆盖常见姓名用字）
    let map: &[(char, &str)] = &[
        ('张', "zhang"), ('王', "wang"), ('李', "li"), ('赵', "zhao"), ('刘', "liu"),
        ('陈', "chen"), ('杨', "yang"), ('黄', "huang"), ('周', "zhou"), ('吴', "wu"),
        ('徐', "xu"), ('孙', "sun"), ('马', "ma"), ('朱', "zhu"), ('胡', "hu"),
        ('郭', "guo"), ('何', "he"), ('高', "gao"), ('林', "lin"), ('罗', "luo"),
        ('郑', "zheng"), ('梁', "liang"), ('谢', "xie"), ('宋', "song"), ('唐', "tang"),
        ('许', "xu"), ('韩', "han"), ('冯', "feng"), ('邓', "deng"), ('曹', "cao"),
        ('彭', "peng"), ('曾', "zeng"), ('肖', "xiao"), ('田', "tian"), ('董', "dong"),
        ('袁', "yuan"), ('潘', "pan"), ('于', "yu"), ('蒋', "jiang"), ('蔡', "cai"),
        ('余', "yu"), ('杜', "du"), ('叶', "ye"), ('程', "cheng"), ('苏', "su"),
        ('魏', "wei"), ('吕', "lv"), ('丁', "ding"), ('任', "ren"), ('沈', "shen"),
        ('姚', "yao"), ('卢', "lu"), ('姜', "jiang"), ('崔', "cui"), ('钟', "zhong"),
        ('谭', "tan"), ('陆', "lu"), ('汪', "wang"), ('范', "fan"), ('金', "jin"),
        ('石', "shi"), ('廖', "liao"), ('贾', "jia"), ('夏', "xia"), ('韦', "wei"),
        ('方', "fang"), ('白', "bai"), ('邹', "zou"), ('孟', "meng"), ('熊', "xiong"),
        ('秦', "qin"), ('邱', "qiu"), ('江', "jiang"), ('尹', "yin"), ('薛', "xue"),
        ('闫', "yan"), ('段', "duan"), ('雷', "lei"), ('侯', "hou"), ('龙', "long"),
        ('史', "shi"), ('陶', "tao"), ('黎', "li"), ('贺', "he"), ('顾', "gu"),
        ('毛', "mao"), ('郝', "hao"), ('龚', "gong"), ('邵', "shao"), ('万', "wan"),
        ('钱', "qian"), ('严', "yan"), ('覃', "qin"), ('武', "wu"), ('戴', "dai"),
        ('莫', "mo"), ('孔', "kong"), ('汤', "tang"), ('哲', "zhe"), ('明', "ming"),
        ('伟', "wei"), ('芳', "fang"), ('娜', "na"), ('敏', "min"), ('静', "jing"),
        ('丽', "li"), ('强', "qiang"), ('磊', "lei"), ('军', "jun"), ('洋', "yang"),
        ('勇', "yong"), ('艳', "yan"), ('杰', "jie"), ('娟', "juan"), ('涛', "tao"),
        ('超', "chao"), ('秀', "xiu"), ('霞', "xia"), ('平', "ping"), ('刚', "gang"),
        ('桂', "gui"), ('文', "wen"), ('辉', "hui"), ('力', "li"), ('建', "jian"),
    ];
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii() && !ch.is_whitespace() {
            out.push(ch.to_ascii_lowercase());
            continue;
        }
        if ch.is_whitespace() {
            continue;
        }
        match map.iter().find(|(c, _)| *c == ch) {
            Some((_, py)) => out.push_str(py),
            None => return None, // 有无法映射的汉字，交给调用方回退
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// 环境变量集合。
///
/// - 内部以 Linux 习惯保存（例如 PATH 用 `:` 分隔，含 HOME/USER）。
/// - 启动或 `export` 时会同步到底层进程环境，保证子进程可见。
/// - `spawn_env()` 返回适合 Windows 子进程使用的环境快照（PATH 转回 `;`）。
pub struct Env {
    vars: HashMap<String, String>,
    last_status: Cell<i32>,
    shell_path: String,
}

impl Env {
    /// 从当前 Windows 进程环境构建初始环境，并注入 Linux 风格变量。
    pub fn new(shell_path: String) -> Self {
        let mut vars: HashMap<String, String> = HashMap::new();
        for (k, v) in std::env::vars() {
            vars.insert(k.to_ascii_uppercase(), v);
        }

        let home = vars
            .get("USERPROFILE")
            .cloned()
            .or_else(|| vars.get("HOME").cloned())
            .unwrap_or_else(|| r"C:\Users\Default".to_string());
        let user = vars
            .get("USERNAME")
            .cloned()
            .or_else(|| vars.get("USER").cloned())
            .unwrap_or_else(|| "user".to_string());

        // USER_ASCII：用于提示符显示，保证只含 ASCII 字符（宽字符会导致
        // 控制台光标定位错误）。优先用拼音映射，失败则回退到 USERDOMAIN/user。
        let user_ascii = to_ascii_name(&user)
            .or_else(|| vars.get("USERDOMAIN").filter(|d| d.is_ascii()).cloned())
            .unwrap_or_else(|| "user".to_string());

        // Linux 风格别名
        vars.insert("HOME".into(), home);
        vars.insert("USER".into(), user.clone());
        vars.insert("LOGNAME".into(), user);
        vars.insert("USER_ASCII".into(), user_ascii);
        vars.insert("SHELL".into(), shell_path.clone());
        vars.entry("LANG".into()).or_insert_with(|| "C.UTF-8".into());
        vars.entry("PWD".into()).or_insert_with(|| {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| r"C:\".into())
        });

        let env = Self {
            vars,
            last_status: Cell::new(0),
            shell_path,
        };
        env.sync_process_env();
        env
    }

    /// 读取变量（Windows 下变量名大小写不敏感）。
    pub fn get(&self, key: &str) -> Option<String> {
        self.vars.get(&key.to_ascii_uppercase()).cloned()
    }

    /// 读取变量，不存在时返回默认值。
    pub fn get_or(&self, key: &str, default: &str) -> String {
        self.get(key).unwrap_or_else(|| default.to_string())
    }

    /// 设置变量（仅内存视图）。
    pub fn set(&mut self, key: &str, value: &str) {
        let k = key.trim();
        if !k.is_empty() {
            self.vars.insert(k.to_ascii_uppercase(), value.to_string());
        }
    }

    /// `export VAR=value`：设置并同步到进程环境，使子进程继承。
    pub fn export(&mut self, key: &str, value: &str) {
        self.set(key, value);
        self.sync_process_env();
    }

    /// 删除变量。
    pub fn unset(&mut self, key: &str) {
        self.vars.remove(&key.to_ascii_uppercase());
        std::env::remove_var(key);
    }

    /// 记录上一条命令的退出码（供 `$?` 使用）。
    pub fn set_last_status(&self, code: i32) {
        self.last_status.set(code);
    }

    pub fn last_status(&self) -> i32 {
        self.last_status.get()
    }

    pub fn shell_path(&self) -> &str {
        &self.shell_path
    }

    pub fn home_path(&self) -> PathBuf {
        PathBuf::from(self.get_or("HOME", r"C:\Users\Default"))
    }

    /// 返回 HOME 的 Windows 路径表示（供 pathutil 使用）。
    pub fn home_string(&self) -> String {
        self.get_or("HOME", r"C:\Users\Default")
    }

    /// 全部变量快照，用于 `env` 命令（按名字排序）。
    pub fn all(&self) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = self
            .vars
            .iter()
            .map(|(k, val)| (k.clone(), val.clone()))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    /// 展开字符串中的变量与 `~`：
    /// `$VAR`、`${VAR}`、`$?`、`$$`、`~`。
    pub fn expand(&self, s: &str) -> String {
        let chars: Vec<char> = s.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            // ~ 在 token 开头时展开
            if chars[i] == '~' && out.is_empty() {
                out.push_str(&self.home_string());
                i += 1;
                continue;
            }
            if chars[i] == '$' && i + 1 < chars.len() {
                match chars[i + 1] {
                    '?' => {
                        out.push_str(&self.last_status().to_string());
                        i += 2;
                        continue;
                    }
                    '$' => {
                        out.push_str(&std::process::id().to_string());
                        i += 2;
                        continue;
                    }
                    '{' => {
                        if let Some(end) = chars[i + 2..].iter().position(|&c| c == '}') {
                            let name: String = chars[i + 2..i + 2 + end].iter().collect();
                            out.push_str(&self.get_or(&name, ""));
                            i += 2 + end + 1;
                            continue;
                        }
                    }
                    c if c.is_ascii_alphabetic() || c == '_' => {
                        let mut j = i + 1;
                        while j < chars.len()
                            && (chars[j].is_ascii_alphanumeric() || chars[j] == '_')
                        {
                            j += 1;
                        }
                        let name: String = chars[i + 1..j].iter().collect();
                        out.push_str(&self.get_or(&name, ""));
                        i = j;
                        continue;
                    }
                    _ => {}
                }
            }
            out.push(chars[i]);
            i += 1;
        }
        out
    }

    /// 生成 Windows 子进程使用的环境快照：PATH 由 `:` 转回 `;`。
    pub fn spawn_env(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for (k, v) in &self.vars {
            let mut val = v.clone();
            if k == "PATH" && val.contains(':') && !val.contains(';') {
                val = val.replace(':', ";");
            }
            map.insert(k.clone(), val);
        }
        map
    }

    /// PATH 的 Linux 风格展示（把 `;` 转成 `:`）。
    pub fn path_display(&self) -> String {
        self.get_or("PATH", "").replace(';', ":")
    }

    /// 把内存视图同步到当前进程环境，使通过 `Command` 启动的进程继承。
    fn sync_process_env(&self) {
        let env = self.spawn_env();
        for (k, v) in env {
            std::env::set_var(k, v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinyi_mapping() {
        assert_eq!(to_ascii_name("张哲"), Some("zhangzhe".into()));
        assert_eq!(to_ascii_name("john"), Some("john".into()));
        assert_eq!(to_ascii_name("李雷"), Some("lilei".into()));
        // 含无法映射的字符时回退
        assert_eq!(to_ascii_name("😀"), None);
    }

    #[test]
    fn user_ascii_is_ascii() {
        let e = Env::new("test".into());
        let ua = e.get_or("USER_ASCII", "");
        assert!(!ua.is_empty());
        assert!(ua.is_ascii(), "USER_ASCII must be ASCII: {}", ua);
    }
}
