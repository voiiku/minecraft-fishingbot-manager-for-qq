//! 指令回复的统一结构。
//!
//! 需要输出报告的指令都用 [`Report`] 组装、由 [`Report::render`] 渲染，
//! 结构固定为「标题 + 基础信息/列表行 + 各类明细 + 整块内容」，未提供的部分不会出现在结果里；
//! 只回一行的单个机器人指令用 [`Report::is_ok`] 与 [`Report::failure_text`] 判断结果。
//!
//! 渲染时按 `runtime.max_reply_chars` 收口长度：协议端对单条消息有长度限制，
//! 超长会**直接发送失败**（用户那边看起来"什么都没发生"），所以宁可截断并写明省略了多少行。
//!
//! 新增输出类型时，在结构体里加一个可选字段并同步更新 `empty()` 与 `render()` 即可，
//! 构造统一走 [`Report::new`] 与 [`Report::untitled`]，因此不会出现"新字段漏赋值"。
//!
//! 另外提供 [`IntoReplyText`]，把业务函数的 `Result<String, String>` 直接转成回复文本。

use crate::config;

/// 通用报告体：标题、基础信息/列表行、各类明细、整块内容，全部可选。
///
/// 约定：列表字段为空即视为未提供；单值字段用 `Option` 表示未提供。
/// 渲染时未提供的部分不会出现。
#[derive(Debug)]
pub struct Report {
    /// 标题；`None` 时省略标题与分隔线。
    pub title: Option<String>,
    /// 基础信息行与列表行：计数、环境指标、表头、名单、状态行等。
    ///
    /// 名单、状态表这类「一行一条」的数据也放这里 —— 它们算报告开头就该看到的信息，
    /// 单独开一节只会把表头与数据行割裂开，因此不再另设明细字段。
    pub basics: Vec<String>,
    /// 成功明细行。
    pub succeeded: Vec<String>,
    /// 跳过明细行，例如「已在运行」「原本未运行」。
    pub skipped: Vec<String>,
    /// 失败明细行。
    pub failed: Vec<String>,
    /// 整块原样内容：日志、配置、帮助文本等。
    pub content: Option<String>,
}

impl Report {
    /// 创建带标题的空报告。
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: Some(title.into()),
            ..Self::empty()
        }
    }

    /// 创建无标题的空报告；标题可用 [`Report::set_title`] 后补。
    pub fn untitled() -> Self {
        Self::empty()
    }

    /// 设置标题；用于标题要等流程跑完才能确定的场景。
    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = Some(title.into());
    }

    /// 用一组「标签 + 数量」覆盖基础信息行，统一格式为「标签: N 个」。
    ///
    /// 批量指令的计数行都这么写：一次给全，顺序与回复里看到的完全一致；
    /// 比"先清空、再逐条追加"更难写漏，也不会把助手预留的占位信息叠加进来。
    /// 非计数的基础信息行（表头、名单、说明）直接往 `basics` 里 push。
    pub fn set_counts(&mut self, counts: &[(&str, usize)]) {
        self.basics = counts
            .iter()
            .map(|(label, count)| format!("{label}: {count} 个"))
            .collect();
    }

    /// 合并另一份报告，用于「停止 + 启动」这类组合流程。
    pub fn absorb(&mut self, other: Report) {
        self.succeeded.extend(other.succeeded);
        self.skipped.extend(other.skipped);
        self.failed.extend(other.failed);
        if self.content.is_none() {
            self.content = other.content;
        }
    }

    /// 是否没有任何失败项。
    pub fn is_ok(&self) -> bool {
        self.failed.is_empty()
    }

    /// 把失败信息拼成一行，供单个机器人指令作为错误提示返回。
    pub fn failure_text(&self) -> String {
        self.failed.join(" / ")
    }

    /// 渲染成 QQ 回复文本；空的部分自动省略，超长部分按 `runtime.max_reply_chars` 截断。
    pub fn render(&self) -> String {
        let mut blocks: Vec<String> = Vec::new();

        let mut header: Vec<String> = Vec::new();
        if let Some(title) = &self.title {
            header.push(format!("{title}:"));
            header.push("=======================".to_string());
        }
        header.extend(self.basics.iter().map(|line| format!("- {line}")));
        if !header.is_empty() {
            blocks.push(header.join("\n"));
        }

        push_list_block(&mut blocks, "成功明细", &self.succeeded);
        push_list_block(&mut blocks, "跳过明细", &self.skipped);
        push_list_block(&mut blocks, "失败明细", &self.failed);

        if let Some(content) = &self.content {
            blocks.push(format!("【内容】:\n{content}"));
        }

        truncate_reply(blocks.join("\n\n"), config::get().runtime.max_reply_chars)
    }

    /// 所有字段的默认取值集中在这里：新增输出类型时只需改这一处。
    fn empty() -> Self {
        Self {
            title: None,
            basics: Vec::new(),
            succeeded: Vec::new(),
            skipped: Vec::new(),
            failed: Vec::new(),
            content: None,
        }
    }
}

/// 追加一个逐行列出的小节；内容为空时跳过。
fn push_list_block(blocks: &mut Vec<String>, label: &str, lines: &[String]) {
    if lines.is_empty() {
        return;
    }

    let mut block = vec![format!("【{label}】:")];
    block.extend(lines.iter().map(|line| format!("- {line}")));
    blocks.push(block.join("\n"));
}

/// 把回复文本按字符上限截断：从**末尾**按行丢，末尾注明省略了多少行。
///
/// 按行截断而不是硬切字符：回复里每一行都是一条独立信息，切半行会让人以为内容坏了；
/// 另一方面标题与计数行在开头，因此截断后最该看到的信息一定还在。
/// 第一行本身就超长时退化成硬切，保证结果一定能收进上限。
fn truncate_reply(reply_text: String, max_chars: usize) -> String {
    if reply_text.chars().count() <= max_chars {
        return reply_text;
    }

    let lines: Vec<&str> = reply_text.lines().collect();
    let mut kept_lines: Vec<&str> = Vec::new();
    let mut used_chars = 0usize;
    for line in &lines {
        // 换行符也算进长度，否则多行拼起来又会重新超限。
        let line_chars = line.chars().count() + 1;
        if used_chars + line_chars > max_chars {
            break;
        }
        kept_lines.push(line);
        used_chars += line_chars;
    }

    if kept_lines.is_empty() {
        let note = "…（内容过长，已截断）";
        let first_line = lines.first().copied().unwrap_or_default();
        let keep_chars = max_chars.saturating_sub(note.chars().count());
        let hard_cut: String = first_line.chars().take(keep_chars).collect();
        return format!("{hard_cut}{note}");
    }

    let dropped_line_count = lines.len() - kept_lines.len();
    format!(
        "{}\n…（内容过长，已省略 {dropped_line_count} 行）",
        kept_lines.join("\n")
    )
}

/// `Result` 到回复文本的转换约定。
///
/// 业务函数返回的 `Err` 会直接回复给用户，因此 `Err` 中不得包含路径、
/// 会话名等内部细节。
pub trait IntoReplyText {
    /// 转换为可直接回复的文本。
    fn to_reply_text(self) -> String;
}

impl IntoReplyText for Result<String, String> {
    fn to_reply_text(self) -> String {
        self.unwrap_or_else(|msg| msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 短回复原样返回；超长时按行截断，并注明省略了多少行（标题保留在最前面）。
    #[test]
    fn long_replies_are_truncated_by_lines() {
        let reply = "标题:\n- 一行\n- 二行\n- 三行".to_string();
        assert_eq!(truncate_reply(reply.clone(), 1000), reply);

        let truncated = truncate_reply(reply, 12);
        assert_eq!(truncated, "标题:\n- 一行\n…（内容过长，已省略 2 行）");
    }

    /// 第一行就超长时（例如某个明细行特别长）硬切，但一定还留一句说明，不能返回空串。
    #[test]
    fn single_overlong_line_is_hard_cut() {
        // 20 个字符的正文 + 15 的上限：正文被切到 4 个字符，剩下位置留给说明。
        let overlong = "一二三四五六七八九十甲乙丙丁戊己庚辛壬癸";
        let truncated = truncate_reply(overlong.to_string(), 15);

        assert!(truncated.starts_with("一二三四"), "{truncated}");
        assert!(truncated.ends_with("…（内容过长，已截断）"), "{truncated}");
        assert_eq!(truncated.chars().count(), 15);
    }
}
