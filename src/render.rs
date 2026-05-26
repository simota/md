use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Debug)]
pub struct RenderOptions {
    pub style: String,
    pub width: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Theme {
    Dark,
    Light,
}

#[derive(Clone, Debug, Default)]
struct InlineStyle {
    strong: bool,
    emphasis: bool,
    code: bool,
    link: bool,
}

#[derive(Clone, Debug)]
struct Span {
    text: String,
    style: InlineStyle,
}

#[derive(Clone, Debug)]
enum Block {
    Paragraph(Vec<Span>),
    Heading {
        level: usize,
        spans: Vec<Span>,
    },
    Code {
        language: Option<String>,
        text: String,
    },
    Quote(Vec<Span>),
    Rule,
    List {
        ordered_start: Option<u64>,
        items: Vec<Vec<Span>>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SpanTarget {
    Paragraph,
    Heading,
    Quote,
    ListItem,
}

#[derive(Clone, Debug, Default)]
struct RenderState {
    blocks: Vec<Block>,
    current_target: Option<SpanTarget>,
    current_spans: Vec<Span>,
    current_heading_level: usize,
    current_code_language: Option<String>,
    current_code: String,
    in_code_block: bool,
    in_quote: bool,
    list_start: Option<u64>,
    list_items: Vec<Vec<Span>>,
    in_list_item: bool,
    style: InlineStyle,
}

pub fn render_markdown(markdown: &str, opts: RenderOptions) -> Result<String, String> {
    let theme = theme_for(&opts.style)?;
    let width = if opts.width == 0 {
        80
    } else {
        opts.width.max(24)
    };
    let blocks = parse_blocks(markdown);
    Ok(render_blocks(&blocks, width, theme))
}

fn parse_blocks(markdown: &str) -> Vec<Block> {
    let parser = Parser::new_ext(
        markdown,
        Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TABLES
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_FOOTNOTES,
    );
    let mut state = RenderState::default();

    for event in parser {
        match event {
            Event::Start(tag) => state.start_tag(tag),
            Event::End(tag) => state.end_tag(tag),
            Event::Text(text) => state.push_text(&text),
            Event::Code(text) => state.push_code_span(&text),
            Event::Html(text) | Event::InlineHtml(text) => state.push_text(&text),
            Event::SoftBreak => state.push_text(" "),
            Event::HardBreak => state.push_text("\n"),
            Event::Rule => {
                state.flush_current();
                state.blocks.push(Block::Rule);
            }
            Event::TaskListMarker(checked) => {
                state.push_text(if checked { "[x] " } else { "[ ] " });
            }
            Event::FootnoteReference(text) => state.push_text(&format!("[{text}]")),
            Event::InlineMath(text) => state.push_code_span(&text),
            Event::DisplayMath(text) => {
                state.flush_current();
                state.blocks.push(Block::Code {
                    language: Some("math".to_string()),
                    text: text.to_string(),
                });
            }
        }
    }
    state.flush_current();
    state.blocks
}

impl RenderState {
    fn start_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                if self.in_list_item {
                    self.current_target = Some(SpanTarget::ListItem);
                } else if self.in_quote {
                    self.current_target = Some(SpanTarget::Quote);
                } else {
                    self.current_target = Some(SpanTarget::Paragraph);
                }
            }
            Tag::Heading { level, .. } => {
                self.flush_current();
                self.current_target = Some(SpanTarget::Heading);
                self.current_heading_level = heading_level_to_usize(level);
            }
            Tag::BlockQuote(_) => {
                self.flush_current();
                self.in_quote = true;
                self.current_target = Some(SpanTarget::Quote);
            }
            Tag::CodeBlock(kind) => {
                self.flush_current();
                self.in_code_block = true;
                self.current_code.clear();
                self.current_code_language = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().map(str::to_string)
                    }
                    CodeBlockKind::Indented => None,
                };
            }
            Tag::List(start) => {
                self.flush_current();
                self.list_start = start;
                self.list_items.clear();
            }
            Tag::Item => {
                self.in_list_item = true;
                self.current_target = Some(SpanTarget::ListItem);
                self.current_spans.clear();
            }
            Tag::Emphasis => self.style.emphasis = true,
            Tag::Strong => self.style.strong = true,
            Tag::Link { .. } => self.style.link = true,
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush_current(),
            TagEnd::Heading(_) => self.flush_current(),
            TagEnd::BlockQuote(_) => {
                self.flush_current();
                self.in_quote = false;
            }
            TagEnd::CodeBlock => {
                self.blocks.push(Block::Code {
                    language: self.current_code_language.take(),
                    text: self.current_code.trim_end_matches('\n').to_string(),
                });
                self.current_code.clear();
                self.in_code_block = false;
            }
            TagEnd::List(_) => {
                self.flush_current();
                self.blocks.push(Block::List {
                    ordered_start: self.list_start.take(),
                    items: std::mem::take(&mut self.list_items),
                });
            }
            TagEnd::Item => {
                self.flush_current();
                self.in_list_item = false;
            }
            TagEnd::Emphasis => self.style.emphasis = false,
            TagEnd::Strong => self.style.strong = false,
            TagEnd::Link => self.style.link = false,
            _ => {}
        }
    }

    fn push_text(&mut self, text: &str) {
        if self.in_code_block {
            self.current_code.push_str(text);
            return;
        }
        if self.current_target.is_none() {
            self.current_target = Some(if self.in_list_item {
                SpanTarget::ListItem
            } else if self.in_quote {
                SpanTarget::Quote
            } else {
                SpanTarget::Paragraph
            });
        }
        self.current_spans.push(Span {
            text: text.to_string(),
            style: self.style.clone(),
        });
    }

    fn push_code_span(&mut self, text: &str) {
        let mut style = self.style.clone();
        style.code = true;
        if self.current_target.is_none() {
            self.current_target = Some(if self.in_list_item {
                SpanTarget::ListItem
            } else if self.in_quote {
                SpanTarget::Quote
            } else {
                SpanTarget::Paragraph
            });
        }
        self.current_spans.push(Span {
            text: text.to_string(),
            style,
        });
    }

    fn flush_current(&mut self) {
        let Some(target) = self.current_target.take() else {
            return;
        };
        let spans = trim_spans(std::mem::take(&mut self.current_spans));
        if spans.is_empty() {
            return;
        }
        match target {
            SpanTarget::Paragraph => self.blocks.push(Block::Paragraph(spans)),
            SpanTarget::Heading => self.blocks.push(Block::Heading {
                level: self.current_heading_level,
                spans,
            }),
            SpanTarget::Quote => self.blocks.push(Block::Quote(spans)),
            SpanTarget::ListItem => self.list_items.push(spans),
        }
    }
}

fn render_blocks(blocks: &[Block], width: usize, theme: Theme) -> String {
    let mut out = String::new();
    let mut first = true;

    for block in blocks {
        if !first {
            out.push('\n');
        }
        match block {
            Block::Paragraph(spans) => render_paragraph(&mut out, spans, width, theme, 0),
            Block::Heading { level, spans } => {
                render_heading(&mut out, *level, spans, width, theme)
            }
            Block::Code { language, text } => {
                render_code_block(&mut out, language.as_deref(), text, width, theme)
            }
            Block::Quote(spans) => render_quote(&mut out, spans, width, theme),
            Block::Rule => {
                out.push_str(&muted(&"-".repeat(width.min(72)), theme));
                out.push('\n');
            }
            Block::List {
                ordered_start,
                items,
            } => render_list(&mut out, *ordered_start, items, width, theme),
        }
        first = false;
    }

    out
}

fn render_heading(out: &mut String, level: usize, spans: &[Span], width: usize, theme: Theme) {
    let text = spans_plain_text(spans);
    match level {
        1 => {
            out.push_str(&accent_bold(&text, theme));
            out.push('\n');
            out.push_str(&accent(
                &"-".repeat(visible_width(&text).min(width).max(3)),
                theme,
            ));
            out.push('\n');
        }
        2 => {
            out.push_str(&accent_bold(&text, theme));
            out.push('\n');
        }
        _ => {
            out.push_str(&muted(&format!("{} ", "#".repeat(level.min(6))), theme));
            out.push_str(&accent_bold(&text, theme));
            out.push('\n');
        }
    }
}

fn render_paragraph(out: &mut String, spans: &[Span], width: usize, theme: Theme, indent: usize) {
    for line in wrap_styled(spans, width.saturating_sub(indent).max(12)) {
        out.push_str(&" ".repeat(indent));
        out.push_str(&render_spans_inline(&line, theme));
        out.push('\n');
    }
}

fn render_quote(out: &mut String, spans: &[Span], width: usize, theme: Theme) {
    for line in wrap_styled(spans, width.saturating_sub(3).max(12)) {
        out.push_str(&muted("> ", theme));
        out.push_str(&render_spans_inline(&line, theme));
        out.push('\n');
    }
}

fn render_list(
    out: &mut String,
    ordered_start: Option<u64>,
    items: &[Vec<Span>],
    width: usize,
    theme: Theme,
) {
    let start = ordered_start.unwrap_or(1);
    for (idx, item) in items.iter().enumerate() {
        let marker = match ordered_start {
            Some(_) => format!("{}.", start + idx as u64),
            None => "-".to_string(),
        };
        let prefix = format!("  {marker} ");
        let continuation = " ".repeat(prefix.len());
        for (line_idx, line) in wrap_styled(item, width.saturating_sub(prefix.len()).max(12))
            .iter()
            .enumerate()
        {
            if line_idx == 0 {
                out.push_str(&muted(&prefix, theme));
            } else {
                out.push_str(&continuation);
            }
            out.push_str(&render_spans_inline(line, theme));
            out.push('\n');
        }
    }
}

fn render_code_block(
    out: &mut String,
    language: Option<&str>,
    text: &str,
    width: usize,
    theme: Theme,
) {
    let content_width = width.saturating_sub(6).max(12);
    let continuation_width = content_width.saturating_sub(2).max(10);
    let language = language.filter(|value| !value.is_empty());

    out.push_str(&code_block_top_border(language, content_width, theme));
    out.push('\n');

    if text.is_empty() {
        render_code_row(out, "", content_width, theme, language);
        out.push('\n');
        out.push_str(&code_block_bottom_border(content_width, theme));
        out.push('\n');
        return;
    }

    for raw in text.lines() {
        let wrapped = wrap_code_line(raw, content_width, continuation_width);
        if wrapped.is_empty() {
            render_code_row(out, "", content_width, theme, language);
            out.push('\n');
            continue;
        }

        for line in wrapped {
            render_code_row(out, &line, content_width, theme, language);
            out.push('\n');
        }
    }

    out.push_str(&code_block_bottom_border(content_width, theme));
    out.push('\n');
}

fn wrap_styled(spans: &[Span], width: usize) -> Vec<Vec<Span>> {
    let mut lines: Vec<Vec<Span>> = Vec::new();
    let mut current: Vec<Span> = Vec::new();
    let mut current_width = 0;

    for span in spans {
        for token in span_tokens(span) {
            if token.text == "\n" {
                if !current.is_empty() {
                    lines.push(trim_spans(current));
                    current = Vec::new();
                    current_width = 0;
                }
                continue;
            }

            let token_width = visible_width(&token.text);
            let needs_space = current_width > 0
                && !token.text.starts_with(' ')
                && (token.style.code || !should_join_without_leading_space(&token.text));
            let added = token_width + usize::from(needs_space);

            if current_width > 0 && current_width + added > width {
                lines.push(trim_spans(current));
                current = Vec::new();
                current_width = 0;
            }

            if current_width > 0 && needs_space {
                current.push(Span {
                    text: " ".to_string(),
                    style: InlineStyle::default(),
                });
                current_width += 1;
            }

            current_width += token_width;
            current.push(token);
        }
    }

    if !current.is_empty() {
        lines.push(trim_spans(current));
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

fn span_tokens(span: &Span) -> Vec<Span> {
    if span.style.code {
        return vec![span.clone()];
    }

    let mut tokens = Vec::new();
    for part in span.text.split_whitespace() {
        tokens.push(Span {
            text: part.to_string(),
            style: span.style.clone(),
        });
    }
    tokens
}

fn trim_spans(spans: Vec<Span>) -> Vec<Span> {
    let mut out = spans;
    while out.first().is_some_and(|span| span.text.trim().is_empty()) {
        out.remove(0);
    }
    while out.last().is_some_and(|span| span.text.trim().is_empty()) {
        out.pop();
    }
    out
}

fn render_spans_inline(spans: &[Span], theme: Theme) -> String {
    let mut out = String::new();
    for span in spans {
        out.push_str(&render_span(span, theme));
    }
    out
}

fn render_span(span: &Span, theme: Theme) -> String {
    if span.style.code {
        return inline_code_color(&span.text, theme);
    }
    if span.style.link {
        return accent(&span.text, theme);
    }
    if span.style.strong {
        return bold(&span.text);
    }
    if span.style.emphasis {
        return muted(&span.text, theme);
    }
    span.text.clone()
}

fn should_join_without_leading_space(text: &str) -> bool {
    matches!(
        text.chars().next(),
        Some('.')
            | Some(',')
            | Some(';')
            | Some(':')
            | Some('!')
            | Some('?')
            | Some(')')
            | Some(']')
    )
}

pub fn parse_heading(line: &str) -> Option<(usize, &str)> {
    let count = line.as_bytes().iter().take_while(|&&b| b == b'#').count();
    if !(1..=6).contains(&count) {
        return None;
    }
    let text = line[count..].trim();
    if text.is_empty() {
        None
    } else {
        Some((count, text))
    }
}

pub fn is_fence_line(line: &str) -> bool {
    line.starts_with("```") || line.starts_with("~~~")
}

pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            i += 2;
            while i < bytes.len() {
                let c = bytes[i];
                i += 1;
                if (0x40..=0x7e).contains(&c) {
                    break;
                }
            }
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn spans_plain_text(spans: &[Span]) -> String {
    spans.iter().map(|span| span.text.as_str()).collect()
}

fn visible_width(input: &str) -> usize {
    UnicodeWidthStr::width(strip_ansi(input).as_str())
}

fn wrap_code_line(input: &str, first_width: usize, continuation_width: usize) -> Vec<String> {
    if input.is_empty() {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let mut rest = input.trim_end();
    let mut first = true;

    while !rest.is_empty() {
        let width = if first {
            first_width
        } else {
            continuation_width
        };
        if visible_width(rest) <= width {
            if first {
                lines.push(rest.to_string());
            } else {
                lines.push(format!("  {rest}"));
            }
            break;
        }

        let split = split_index_for_width(rest, width);
        let (line, next) = rest.split_at(split);
        if first {
            lines.push(line.trim_end().to_string());
        } else {
            lines.push(format!("  {}", line.trim_end()));
        }
        rest = next.trim_start();
        first = false;
    }

    lines
}

fn split_index_for_width(input: &str, width: usize) -> usize {
    let mut used = 0;
    let mut fallback = 0;
    let mut break_at = None;

    for (idx, ch) in input.char_indices() {
        let ch_width = UnicodeWidthStr::width(ch.to_string().as_str());
        if used + ch_width > width {
            break;
        }
        used += ch_width;
        fallback = idx + ch.len_utf8();

        if ch.is_whitespace() || matches!(ch, '/' | '-' | '_' | '&' | '?' | '=') {
            break_at = Some(idx + ch.len_utf8());
        }
    }

    break_at.filter(|idx| *idx > 0).unwrap_or(fallback.max(1))
}

fn heading_level_to_usize(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn theme_for(style: &str) -> Result<Theme, String> {
    match style.trim().to_ascii_lowercase().as_str() {
        "" | "auto" | "dark" => Ok(Theme::Dark),
        "light" => Ok(Theme::Light),
        _ => Err(format!("invalid --style={style:?} (use auto|dark|light)")),
    }
}

fn bold(text: &str) -> String {
    format!("\x1b[1m{text}\x1b[0m")
}

fn accent(text: &str, theme: Theme) -> String {
    let color = match theme {
        Theme::Dark => "\x1b[38;2;138;180;248m",
        Theme::Light => "\x1b[38;2;37;99;235m",
    };
    format!("{color}{text}\x1b[0m")
}

fn accent_bold(text: &str, theme: Theme) -> String {
    let color = match theme {
        Theme::Dark => "\x1b[38;2;138;180;248m",
        Theme::Light => "\x1b[38;2;37;99;235m",
    };
    format!("{color}\x1b[1m{text}\x1b[0m")
}

fn muted(text: &str, theme: Theme) -> String {
    let color = match theme {
        Theme::Dark => "\x1b[38;2;184;184;184m",
        Theme::Light => "\x1b[38;2;80;80;80m",
    };
    format!("{color}{text}\x1b[0m")
}

fn command_color(text: &str, theme: Theme) -> String {
    let color = match theme {
        Theme::Dark => "\x1b[38;2;255;107;107m",
        Theme::Light => "\x1b[38;2;190;24;24m",
    };
    format!("{color}\x1b[1m{text}\x1b[0m")
}

fn keyword_color(text: &str, theme: Theme) -> String {
    let color = match theme {
        Theme::Dark => "\x1b[38;2;255;107;107m",
        Theme::Light => "\x1b[38;2;190;24;24m",
    };
    format!("{color}{text}\x1b[0m")
}

fn flag_color(text: &str, theme: Theme) -> String {
    let color = match theme {
        Theme::Dark => "\x1b[38;2;255;209;102m",
        Theme::Light => "\x1b[38;2;146;64;14m",
    };
    format!("{color}{text}\x1b[0m")
}

fn shell_text_color(text: &str, theme: Theme) -> String {
    let color = match theme {
        Theme::Dark => "\x1b[38;2;255;160;160m",
        Theme::Light => "\x1b[38;2;153;27;27m",
    };
    format!("{color}{text}\x1b[0m")
}

fn string_color(text: &str, theme: Theme) -> String {
    let color = match theme {
        Theme::Dark => "\x1b[38;2;126;231;135m",
        Theme::Light => "\x1b[38;2;21;128;61m",
    };
    format!("{color}{text}\x1b[0m")
}

fn comment_color(text: &str, theme: Theme) -> String {
    let color = match theme {
        Theme::Dark => "\x1b[38;2;128;139;150m",
        Theme::Light => "\x1b[38;2;106;115;125m",
    };
    format!("{color}{text}\x1b[0m")
}

fn inline_code_color(text: &str, theme: Theme) -> String {
    let color = match theme {
        Theme::Dark => "\x1b[38;2;255;160;160m",
        Theme::Light => "\x1b[38;2;153;27;27m",
    };
    format!("{color}{text}\x1b[0m")
}

fn code_block_text(text: &str, theme: Theme) -> String {
    let color = match theme {
        Theme::Dark => "\x1b[38;2;230;230;230m",
        Theme::Light => "\x1b[38;2;26;26;26m",
    };
    format!("{color}{text}\x1b[0m")
}

fn code_block_top_border(language: Option<&str>, content_width: usize, theme: Theme) -> String {
    let inner_width = content_width + 2;
    let label = language
        .map(|value| format!(" {value} "))
        .unwrap_or_default();
    let label_width = visible_width(&label);
    let dash_count = inner_width.saturating_sub(label_width).max(3);
    muted(&format!("  +{label}{}+", "-".repeat(dash_count)), theme)
}

fn code_block_bottom_border(content_width: usize, theme: Theme) -> String {
    muted(&format!("  +{}+", "-".repeat(content_width + 2)), theme)
}

fn render_code_row(
    out: &mut String,
    text: &str,
    content_width: usize,
    theme: Theme,
    language: Option<&str>,
) {
    let text = fit_code_cell(text, content_width);
    let padding = " ".repeat(content_width.saturating_sub(visible_width(&text)));
    out.push_str(&muted("  | ", theme));
    out.push_str(&highlight_code_line(&text, language, theme));
    out.push_str(&padding);
    out.push_str(&muted(" |", theme));
}

fn fit_code_cell(text: &str, width: usize) -> String {
    if visible_width(text) <= width {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let ch_width = UnicodeWidthStr::width(ch.to_string().as_str());
        if visible_width(&out) + ch_width > width {
            break;
        }
        out.push(ch);
    }
    out
}

fn highlight_code_line(text: &str, language: Option<&str>, theme: Theme) -> String {
    let lang = language.unwrap_or_default().to_ascii_lowercase();
    if matches!(lang.as_str(), "bash" | "sh" | "shell" | "zsh") {
        return highlight_shell_line(text, theme);
    }
    if lang.is_empty() && looks_like_shell_line(text) {
        return highlight_shell_line(text, theme);
    }
    if matches!(lang.as_str(), "rust" | "rs") {
        return highlight_rust_line(text, theme);
    }
    code_block_text(text, theme)
}

fn highlight_shell_line(text: &str, theme: Theme) -> String {
    let trimmed = text.trim_start();
    if trimmed.starts_with('#') {
        return comment_color(text, theme);
    }

    let mut out = String::new();
    let mut token_start = !text.starts_with("  ");
    for token in shell_tokens_preserving_space(text) {
        if token.trim().is_empty() {
            out.push_str(&token);
            continue;
        }
        if token_start {
            out.push_str(&command_color(&token, theme));
            token_start = false;
        } else if token.starts_with('-') {
            out.push_str(&flag_color(&token, theme));
        } else if token.starts_with('"') || token.starts_with('\'') {
            out.push_str(&string_color(&token, theme));
        } else if token.contains("://") {
            out.push_str(&accent(&token, theme));
        } else {
            out.push_str(&shell_text_color(&token, theme));
        }
    }
    out
}

fn looks_like_shell_line(text: &str) -> bool {
    let trimmed = text.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return true;
    }
    let Some(command) = trimmed.split_whitespace().next() else {
        return false;
    };
    matches!(
        command,
        "cat"
            | "cd"
            | "chmod"
            | "cp"
            | "curl"
            | "echo"
            | "git"
            | "go"
            | "cargo"
            | "make"
            | "mkdir"
            | "mv"
            | "npm"
            | "pnpm"
            | "rm"
            | "rustup"
            | "sh"
            | "sudo"
            | "tar"
            | "unzip"
            | "yarn"
    ) || command.contains('=')
        || command.starts_with("./")
}

fn highlight_rust_line(text: &str, theme: Theme) -> String {
    let trimmed = text.trim_start();
    if trimmed.starts_with("//") {
        return comment_color(text, theme);
    }

    let mut out = String::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            current.push(ch);
            continue;
        }
        flush_rust_token(&mut out, &current, theme);
        current.clear();
        out.push_str(&code_block_text(&ch.to_string(), theme));
    }
    flush_rust_token(&mut out, &current, theme);
    out
}

fn flush_rust_token(out: &mut String, token: &str, theme: Theme) {
    if token.is_empty() {
        return;
    }
    if matches!(
        token,
        "fn" | "let"
            | "mut"
            | "pub"
            | "struct"
            | "enum"
            | "impl"
            | "use"
            | "mod"
            | "match"
            | "if"
            | "else"
            | "for"
            | "while"
            | "loop"
            | "return"
    ) {
        out.push_str(&keyword_color(token, theme));
    } else {
        out.push_str(&code_block_text(token, theme));
    }
}

fn shell_tokens_preserving_space(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut current_is_space = None;

    for ch in text.chars() {
        let is_space = ch.is_whitespace();
        match current_is_space {
            Some(kind) if kind == is_space => current.push(ch),
            Some(_) => {
                tokens.push(current);
                current = ch.to_string();
                current_is_space = Some(is_space);
            }
            None => {
                current.push(ch);
                current_is_space = Some(is_space);
            }
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_markdown_non_empty() {
        let out = render_markdown(
            "# Title\n\nHello\n",
            RenderOptions {
                style: "auto".to_string(),
                width: 60,
            },
        )
        .unwrap();
        assert!(!out.is_empty());
        assert!(strip_ansi(&out).contains("Title"));
    }

    #[test]
    fn parse_heading_requires_text() {
        assert_eq!(parse_heading("# Title"), Some((1, "Title")));
        assert_eq!(parse_heading("####### no"), None);
        assert_eq!(parse_heading("#"), None);
    }

    #[test]
    fn render_keeps_inline_code_span_together() {
        let out = render_markdown(
            "Current dependencies require Rust `>= 1.80`.",
            RenderOptions {
                style: "light".to_string(),
                width: 38,
            },
        )
        .unwrap();
        assert!(strip_ansi(&out).contains(">= 1.80"));
        assert!(out.contains("\x1b[38;2;153;27;27m>= 1.80\x1b[0m"));
        assert!(!out.contains("\x1b[48;2;242;242;242m>= 1.80"));
    }

    #[test]
    fn render_structures_lists_quotes_and_code() {
        let out = render_markdown(
            "- one\n- **two**\n\n> quote\n\n```rust\nfn main() {}\n```\n",
            RenderOptions {
                style: "dark".to_string(),
                width: 50,
            },
        )
        .unwrap();
        let plain = strip_ansi(&out);
        assert!(plain.contains("- one"));
        assert!(plain.contains("> quote"));
        assert!(plain.contains("rust"));
        assert!(plain.contains("fn main() {}"));
    }

    #[test]
    fn render_code_block_wraps_without_truncating() {
        let out = render_markdown(
            "```bash\ncurl -fL -o asset.tar.gz https://example.com/releases/download/v0.1.0/asset.tar.gz\n```\n",
            RenderOptions {
                style: "dark".to_string(),
                width: 48,
            },
        )
        .unwrap();
        let plain = strip_ansi(&out);
        assert!(plain.contains("bash"));
        assert!(plain.contains("curl -fL -o asset.tar.gz"));
        assert!(plain.contains("https://"));
        assert!(plain.contains("example.com/releases/download/v0.1.0/"));
        assert!(plain.contains("asset.tar.gz"));
        assert!(plain.contains("+"));
        assert!(plain.contains("|"));
        assert!(!plain.contains("..."));
    }

    #[test]
    fn shell_code_block_highlights_command_in_red() {
        let out = render_markdown(
            "```bash\ncurl -fL https://example.com\n```\n",
            RenderOptions {
                style: "dark".to_string(),
                width: 48,
            },
        )
        .unwrap();
        assert!(out.contains("\x1b[38;2;255;107;107m\x1b[1mcurl\x1b[0m"));
    }

    #[test]
    fn shell_code_block_colors_plain_arguments_red_too() {
        let out = render_markdown(
            "```bash\nmake build\n```\n",
            RenderOptions {
                style: "dark".to_string(),
                width: 48,
            },
        )
        .unwrap();
        assert!(out.contains("\x1b[38;2;255;160;160mbuild\x1b[0m"));
    }

    #[test]
    fn unlabeled_shell_like_code_block_is_highlighted() {
        let out = render_markdown(
            "```\nmake build\n```\n",
            RenderOptions {
                style: "dark".to_string(),
                width: 48,
            },
        )
        .unwrap();
        assert!(out.contains("\x1b[38;2;255;107;107m\x1b[1mmake\x1b[0m"));
    }

    #[test]
    fn shell_continuation_line_is_not_colored_as_command() {
        let out = render_markdown(
            "```bash\ncurl https://example.com/releases/download/v0.1.0/asset.tar.gz\n```\n",
            RenderOptions {
                style: "dark".to_string(),
                width: 40,
            },
        )
        .unwrap();
        assert!(!out.contains("\x1b[38;2;255;107;107m\x1b[1mdownload"));
    }
}
