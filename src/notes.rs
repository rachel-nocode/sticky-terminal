use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Clone, Debug)]
pub(crate) struct ParsedMarkdownLine {
    pub(crate) kind: LineKind,
    pub(crate) text: String,
    pub(crate) indent: usize,
    pub(crate) checked: Option<bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LineKind {
    Heading(u8),
    CheckboxItem,
    BulletItem,
    CodeBlock,
    Separator,
    Plain,
}

pub(crate) fn parse_markdown(text: &str) -> Vec<ParsedMarkdownLine> {
    let mut lines = Vec::new();
    let mut in_code_block = false;

    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            lines.push(ParsedMarkdownLine {
                kind: LineKind::CodeBlock,
                text: line.to_owned(),
                indent: 0,
                checked: None,
            });
            continue;
        }

        if in_code_block {
            lines.push(ParsedMarkdownLine {
                kind: LineKind::CodeBlock,
                text: line.to_owned(),
                indent: 0,
                checked: None,
            });
            continue;
        }

        let indent = indent_level(line);
        let trimmed = line.trim_start();

        if trimmed.starts_with("# ") {
            lines.push(ParsedMarkdownLine { kind: LineKind::Heading(1), text: trimmed[2..].to_owned(), indent, checked: None });
        } else if trimmed.starts_with("## ") {
            lines.push(ParsedMarkdownLine { kind: LineKind::Heading(2), text: trimmed[3..].to_owned(), indent, checked: None });
        } else if trimmed.starts_with("### ") {
            lines.push(ParsedMarkdownLine { kind: LineKind::Heading(3), text: trimmed[4..].to_owned(), indent, checked: None });
        } else if trimmed == "---" || trimmed == "***" || trimmed == "___" {
            lines.push(ParsedMarkdownLine { kind: LineKind::Separator, text: String::new(), indent, checked: None });
        } else if trimmed.starts_with("- [ ] ") {
            lines.push(ParsedMarkdownLine { kind: LineKind::CheckboxItem, text: trimmed[6..].to_owned(), indent, checked: Some(false) });
        } else if trimmed.starts_with("- [x] ") || trimmed.starts_with("- [X] ") {
            lines.push(ParsedMarkdownLine { kind: LineKind::CheckboxItem, text: trimmed[6..].to_owned(), indent, checked: Some(true) });
        } else if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            lines.push(ParsedMarkdownLine { kind: LineKind::BulletItem, text: trimmed[2..].to_owned(), indent, checked: None });
        } else {
            lines.push(ParsedMarkdownLine { kind: LineKind::Plain, text: line.to_owned(), indent, checked: None });
        }
    }

    lines
}

fn indent_level(line: &str) -> usize {
    let leading: usize = line
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .map(|c| if c == '\t' { 2 } else { 1 })
        .sum();
    leading / 2
}

pub(crate) fn hash_string(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}
