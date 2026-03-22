use std::hash::{Hash, Hasher};

pub(crate) fn hash_string(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Pre-parsed representation of a single markdown block/line.
/// Built once when content changes; rendered from each frame without re-parsing.
#[derive(Clone)]
pub(crate) enum ParsedMarkdownLine {
    Empty,
    H3(String),
    H2(String),
    H1(String),
    CheckedTask { text: String, line_idx: usize, indent: usize },
    UncheckedTask { text: String, line_idx: usize, indent: usize },
    Bullet { text: String, indent: usize },
    Numbered { num: String, text: String, indent: usize },
    Blockquote(String),
    CodeBlock(String),
    Rule,
    Paragraph { text: String, indent: usize },
}

/// Parse markdown text into a `Vec<ParsedMarkdownLine>`.
/// Call this only when the content hash changes; cache the result.
pub(crate) fn parse_markdown(markdown: &str) -> Vec<ParsedMarkdownLine> {
    let lines: Vec<&str> = markdown.lines().collect();
    let mut result = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        let ind = indent_level(line);

        if trimmed.is_empty() {
            result.push(ParsedMarkdownLine::Empty);
        } else if let Some(t) = trimmed.strip_prefix("### ") {
            result.push(ParsedMarkdownLine::H3(t.to_owned()));
        } else if let Some(t) = trimmed.strip_prefix("## ") {
            result.push(ParsedMarkdownLine::H2(t.to_owned()));
        } else if let Some(t) = trimmed.strip_prefix("# ") {
            result.push(ParsedMarkdownLine::H1(t.to_owned()));
        } else if trimmed.starts_with("- [x] ") || trimmed.starts_with("- [X] ") {
            result.push(ParsedMarkdownLine::CheckedTask {
                text: trimmed[6..].to_owned(),
                line_idx: i,
                indent: ind,
            });
        } else if trimmed.starts_with("- [ ] ") {
            result.push(ParsedMarkdownLine::UncheckedTask {
                text: trimmed[6..].to_owned(),
                line_idx: i,
                indent: ind,
            });
        } else if let Some(t) =
            trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* "))
        {
            result.push(ParsedMarkdownLine::Bullet {
                text: t.to_owned(),
                indent: ind,
            });
        } else if let Some((num, text)) = trimmed
            .split_once(". ")
            .filter(|(n, _)| !n.is_empty() && n.chars().all(|ch| ch.is_ascii_digit()))
        {
            result.push(ParsedMarkdownLine::Numbered {
                num: num.to_owned(),
                text: text.to_owned(),
                indent: ind,
            });
        } else if let Some(t) = trimmed.strip_prefix("> ") {
            result.push(ParsedMarkdownLine::Blockquote(t.to_owned()));
        } else if trimmed.starts_with("```") {
            let mut code_lines: Vec<&str> = Vec::new();
            i += 1;
            while i < lines.len() {
                if lines[i].trim().starts_with("```") {
                    break;
                }
                code_lines.push(lines[i]);
                i += 1;
            }
            let code_block = if code_lines.is_empty() {
                " ".to_owned()
            } else {
                code_lines.join("\n")
            };
            result.push(ParsedMarkdownLine::CodeBlock(code_block));
        } else if trimmed == "---" || trimmed == "***" || trimmed == "___" {
            result.push(ParsedMarkdownLine::Rule);
        } else {
            result.push(ParsedMarkdownLine::Paragraph {
                text: trimmed.to_owned(),
                indent: ind,
            });
        }
        i += 1;
    }
    result
}

/// Calculate indent level from leading whitespace (each 2 spaces or 1 tab = 1 level)
pub(crate) fn indent_level(line: &str) -> usize {
    let leading: usize = line
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .map(|c| if c == '\t' { 2 } else { 1 })
        .sum();
    leading / 2
}
