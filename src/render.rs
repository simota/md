//! Markdown -> ANSI terminal renderer.
//!
//! The renderer works in two phases:
//!
//! 1. `parse_blocks` turns pulldown-cmark events into a small block tree
//!    (paragraphs, headings, nested lists/quotes, tables, code, ...).
//! 2. `render_blocks` lays the tree out for a given width and produces
//!    ANSI-styled lines plus metadata (heading / link / code block positions)
//!    that the pager uses for navigation.

use std::collections::HashMap;

use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Clone, Debug)]
pub struct RenderOptions {
    pub style: String,
    pub width: usize,
}

/// A heading as it appears in the rendered output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadingInfo {
    pub level: usize,
    pub text: String,
    /// GitHub-style anchor slug (unique within the document).
    pub anchor: String,
    /// Index of the rendered line the heading starts on.
    pub line: usize,
}

/// A link or image target found in the document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkInfo {
    pub text: String,
    pub url: String,
    pub image: bool,
    /// Index of the rendered line where the link text starts.
    pub line: usize,
}

/// A fenced/indented code block and where it was rendered.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodeBlockInfo {
    pub language: Option<String>,
    pub code: String,
    pub start_line: usize,
    pub end_line: usize,
}

/// Rendered document: ANSI lines plus navigation metadata.
#[derive(Clone, Debug, Default)]
pub struct Rendered {
    pub lines: Vec<String>,
    pub headings: Vec<HeadingInfo>,
    pub links: Vec<LinkInfo>,
    pub code_blocks: Vec<CodeBlockInfo>,
}

impl Rendered {
    /// Full ANSI text with a trailing newline (empty for empty documents).
    pub fn text(&self) -> String {
        let mut out = String::with_capacity(self.lines.iter().map(|l| l.len() + 1).sum());
        for line in &self.lines {
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    /// Rendered line range `[start, end)` of the section that starts at
    /// heading `idx` (up to the next heading of the same or higher level).
    pub fn section_range(&self, idx: usize) -> Option<(usize, usize)> {
        let heading = self.headings.get(idx)?;
        let end = self.headings[idx + 1..]
            .iter()
            .find(|h| h.level <= heading.level)
            .map(|h| h.line)
            .unwrap_or(self.lines.len());
        let mut end = end.max(heading.line + 1);
        while end > heading.line + 1 && strip_ansi(&self.lines[end - 1]).trim().is_empty() {
            end -= 1;
        }
        Some((heading.line, end))
    }

    /// Find a heading by anchor slug (exact) or by case-insensitive text
    /// (exact first, then substring).
    pub fn find_heading(&self, query: &str) -> Option<usize> {
        let query = query.trim().trim_start_matches('#');
        if query.is_empty() {
            return None;
        }
        let lower = query.to_lowercase();
        let slug = slugify(query);
        self.headings
            .iter()
            .position(|h| h.anchor == query || h.anchor == lower)
            .or_else(|| {
                self.headings
                    .iter()
                    .position(|h| h.text.to_lowercase() == lower)
            })
            .or_else(|| self.headings.iter().position(|h| h.anchor == slug))
            .or_else(|| {
                self.headings
                    .iter()
                    .position(|h| h.text.to_lowercase().contains(&lower))
            })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Theme {
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct InlineStyle {
    strong: bool,
    emphasis: bool,
    strike: bool,
    code: bool,
    accent: bool,
    muted: bool,
    /// The " (url)" suffix printed after a link's text.
    url: bool,
    link: Option<u32>,
}

#[derive(Clone, Debug)]
struct Span {
    text: String,
    style: InlineStyle,
}

#[derive(Clone, Debug)]
struct ListItem {
    task: Option<bool>,
    blocks: Vec<Block>,
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
    Quote {
        kind: Option<BlockQuoteKind>,
        blocks: Vec<Block>,
    },
    Rule,
    List {
        ordered_start: Option<u64>,
        tight: bool,
        items: Vec<ListItem>,
    },
    Table {
        aligns: Vec<Alignment>,
        header: Vec<Vec<Span>>,
        rows: Vec<Vec<Vec<Span>>>,
    },
    Footnote {
        label: String,
        blocks: Vec<Block>,
    },
    Html(String),
}

pub fn render_markdown(markdown: &str, opts: RenderOptions) -> Result<String, String> {
    Ok(render_document(markdown, &opts)?.text())
}

/// Render markdown into lines plus navigation metadata.
pub fn render_document(markdown: &str, opts: &RenderOptions) -> Result<Rendered, String> {
    let theme = theme_for(&opts.style)?;
    let width = if opts.width == 0 {
        80
    } else {
        opts.width.max(24)
    };
    let markdown = sanitize_control(markdown);
    let parsed = parse_blocks(&markdown);
    let ctx = Ctx { theme, depth: 0 };
    let out = render_blocks(&parsed.blocks, width, ctx, false);

    let mut anchors: HashMap<String, usize> = HashMap::new();
    let headings = out
        .headings
        .into_iter()
        .map(|mut h| {
            let base = slugify(&h.text);
            let n = anchors.entry(base.clone()).or_insert(0);
            h.anchor = if *n == 0 {
                base.clone()
            } else {
                format!("{base}-{n}")
            };
            *n += 1;
            h
        })
        .collect();

    let mut link_lines: HashMap<u32, usize> = HashMap::new();
    for (id, line) in out.links {
        link_lines.entry(id).or_insert(line);
    }
    let links = parsed
        .links
        .into_iter()
        .enumerate()
        .filter_map(|(id, (text, url, image))| {
            link_lines.get(&(id as u32)).map(|line| LinkInfo {
                text,
                url,
                image,
                line: *line,
            })
        })
        .collect();

    Ok(Rendered {
        lines: out.lines,
        headings,
        links,
        code_blocks: out.code,
    })
}

/// Replace terminal control characters (ESC, BEL, C1 controls, ...) with
/// caret notation so document content cannot inject escape sequences
/// (e.g. rewrite the clipboard via OSC 52). Tabs and newlines are kept.
pub fn sanitize_control(input: &str) -> std::borrow::Cow<'_, str> {
    let is_bad = |c: char| {
        (c.is_control() && !matches!(c, '\n' | '\t' | '\r')) || ('\u{80}'..='\u{9f}').contains(&c)
    };
    // Fast byte scan, same result as `input.chars().any(is_bad)`: C0
    // controls and DEL are single bytes, and C1 controls (U+0080..=U+009F)
    // are always encoded as 0xC2 0x80..=0x9F.
    let bytes = input.as_bytes();
    let has_bad = bytes.iter().enumerate().any(|(idx, &b)| {
        (b < 0x20 && !matches!(b, b'\n' | b'\t' | b'\r'))
            || b == 0x7f
            || (b == 0xc2
                && bytes
                    .get(idx + 1)
                    .is_some_and(|n| (0x80..=0x9f).contains(n)))
    });
    if !has_bad {
        return std::borrow::Cow::Borrowed(input);
    }
    let mut out = String::with_capacity(input.len() + 8);
    for c in input.chars() {
        if !is_bad(c) {
            out.push(c);
        } else if (c as u32) < 0x20 {
            out.push('^');
            out.push(char::from(b'@' + c as u8));
        } else if c == '\u{7f}' {
            out.push_str("^?");
        } else {
            out.push_str(&format!("<U+{:04X}>", c as u32));
        }
    }
    std::borrow::Cow::Owned(out)
}

/// GitHub-compatible heading anchor slug.
pub fn slugify(text: &str) -> String {
    let mut out = String::new();
    for ch in text.trim().chars() {
        if ch.is_alphanumeric() || ch == '_' || ch == '-' {
            out.extend(ch.to_lowercase());
        } else if ch == ' ' {
            out.push('-');
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

struct Parsed {
    blocks: Vec<Block>,
    links: Vec<(String, String, bool)>,
}

enum Frame {
    Root(Vec<Block>),
    Quote(Option<BlockQuoteKind>, Vec<Block>),
    List {
        start: Option<u64>,
        tight: bool,
        items: Vec<ListItem>,
    },
    Item {
        task: Option<bool>,
        blocks: Vec<Block>,
    },
    Footnote {
        label: String,
        blocks: Vec<Block>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InlineTarget {
    Paragraph,
    Heading(usize),
    Cell,
}

#[derive(Default)]
struct TableBuild {
    aligns: Vec<Alignment>,
    header: Vec<Vec<Span>>,
    rows: Vec<Vec<Vec<Span>>>,
    row: Vec<Vec<Span>>,
}

struct Builder {
    stack: Vec<Frame>,
    target: Option<InlineTarget>,
    spans: Vec<Span>,
    style: InlineStyle,
    in_code: bool,
    code_language: Option<String>,
    code: String,
    in_html: bool,
    html: String,
    table: Option<TableBuild>,
    /// (link id, url, index into `spans` where the link text starts)
    link_stack: Vec<(u32, String, usize)>,
    links: Vec<(String, String, bool)>,
    image: Option<(String, String)>,
}

fn parse_blocks(markdown: &str) -> Parsed {
    let parser = Parser::new_ext(
        markdown,
        Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TABLES
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_FOOTNOTES
            | Options::ENABLE_GFM,
    );
    let mut b = Builder {
        stack: vec![Frame::Root(Vec::new())],
        target: None,
        spans: Vec::new(),
        style: InlineStyle::default(),
        in_code: false,
        code_language: None,
        code: String::new(),
        in_html: false,
        html: String::new(),
        table: None,
        link_stack: Vec::new(),
        links: Vec::new(),
        image: None,
    };

    for event in parser {
        match event {
            Event::Start(tag) => b.start_tag(tag),
            Event::End(tag) => b.end_tag(tag),
            Event::Text(text) => b.push_text(&text),
            Event::Code(text) => {
                if let Some((alt, _)) = b.image.as_mut() {
                    alt.push_str(&text);
                } else {
                    let mut style = b.style;
                    style.code = true;
                    b.push_span(&text, style);
                }
            }
            Event::Html(text) => {
                if b.in_html {
                    b.html.push_str(&text);
                }
            }
            Event::InlineHtml(text) => {
                let tag = text.trim().to_ascii_lowercase();
                if tag.starts_with("<br") {
                    b.push_text("\n");
                }
            }
            Event::SoftBreak => b.push_text(" "),
            Event::HardBreak => b.push_text("\n"),
            Event::Rule => {
                b.flush_inline();
                b.push_block(Block::Rule);
            }
            Event::TaskListMarker(checked) => {
                for frame in b.stack.iter_mut().rev() {
                    if let Frame::Item { task, .. } = frame {
                        *task = Some(checked);
                        break;
                    }
                }
            }
            Event::FootnoteReference(label) => {
                let style = InlineStyle {
                    accent: true,
                    ..b.style
                };
                b.push_span(&format!("[{label}]"), style);
            }
            Event::InlineMath(text) => {
                let mut style = b.style;
                style.code = true;
                b.push_span(&text, style);
            }
            Event::DisplayMath(text) => {
                b.flush_inline();
                b.push_block(Block::Code {
                    language: Some("math".to_string()),
                    text: text.to_string(),
                });
            }
        }
    }
    b.flush_inline();
    while b.stack.len() > 1 {
        b.pop_frame();
    }
    let blocks = match b.stack.pop() {
        Some(Frame::Root(blocks)) => blocks,
        _ => Vec::new(),
    };
    Parsed {
        blocks,
        links: b.links,
    }
}

impl Builder {
    fn start_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                self.flush_inline();
                if let [.., Frame::List { tight, .. }, Frame::Item { .. }] =
                    self.stack.as_mut_slice()
                {
                    *tight = false;
                }
                self.target = Some(InlineTarget::Paragraph);
            }
            Tag::Heading { level, .. } => {
                self.flush_inline();
                self.target = Some(InlineTarget::Heading(heading_level_to_usize(level)));
            }
            Tag::BlockQuote(kind) => {
                self.flush_inline();
                self.stack.push(Frame::Quote(kind, Vec::new()));
            }
            Tag::CodeBlock(kind) => {
                self.flush_inline();
                self.in_code = true;
                self.code.clear();
                self.code_language = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split(|c: char| c.is_whitespace() || c == ',' || c == '{')
                        .next()
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                    CodeBlockKind::Indented => None,
                };
            }
            Tag::HtmlBlock => {
                self.flush_inline();
                self.in_html = true;
                self.html.clear();
            }
            Tag::List(start) => {
                self.flush_inline();
                self.stack.push(Frame::List {
                    start,
                    tight: true,
                    items: Vec::new(),
                });
            }
            Tag::Item => {
                self.flush_inline();
                self.stack.push(Frame::Item {
                    task: None,
                    blocks: Vec::new(),
                });
            }
            Tag::FootnoteDefinition(label) => {
                self.flush_inline();
                self.stack.push(Frame::Footnote {
                    label: label.to_string(),
                    blocks: Vec::new(),
                });
            }
            Tag::Table(aligns) => {
                self.flush_inline();
                self.table = Some(TableBuild {
                    aligns,
                    ..TableBuild::default()
                });
            }
            Tag::TableCell => {
                self.spans.clear();
                self.target = Some(InlineTarget::Cell);
            }
            Tag::Emphasis => self.style.emphasis = true,
            Tag::Strong => self.style.strong = true,
            Tag::Strikethrough => self.style.strike = true,
            Tag::Link { dest_url, .. } => {
                let id = self.links.len() as u32;
                self.links
                    .push((String::new(), dest_url.to_string(), false));
                self.link_stack
                    .push((id, dest_url.to_string(), self.spans.len()));
                self.style.link = Some(id);
            }
            Tag::Image { dest_url, .. } => {
                self.image = Some((String::new(), dest_url.to_string()));
            }
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) => self.flush_inline(),
            TagEnd::BlockQuote(_) | TagEnd::List(_) | TagEnd::Item | TagEnd::FootnoteDefinition => {
                self.flush_inline();
                self.pop_frame();
            }
            TagEnd::CodeBlock => {
                let text = std::mem::take(&mut self.code);
                let language = self.code_language.take();
                self.push_block(Block::Code {
                    language,
                    text: text.trim_end_matches('\n').to_string(),
                });
                self.in_code = false;
            }
            TagEnd::HtmlBlock => {
                self.in_html = false;
                let html = std::mem::take(&mut self.html);
                if !is_html_comment_only(&html) {
                    self.push_block(Block::Html(html.trim_end().to_string()));
                }
            }
            TagEnd::TableCell => {
                let spans = trim_spans(std::mem::take(&mut self.spans));
                self.target = None;
                if let Some(table) = self.table.as_mut() {
                    table.row.push(spans);
                }
            }
            TagEnd::TableHead => {
                if let Some(table) = self.table.as_mut() {
                    table.header = std::mem::take(&mut table.row);
                }
            }
            TagEnd::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    let row = std::mem::take(&mut table.row);
                    table.rows.push(row);
                }
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    self.push_block(Block::Table {
                        aligns: table.aligns,
                        header: table.header,
                        rows: table.rows,
                    });
                }
            }
            TagEnd::Emphasis => self.style.emphasis = false,
            TagEnd::Strong => self.style.strong = false,
            TagEnd::Strikethrough => self.style.strike = false,
            TagEnd::Link => {
                if let Some((id, url, start)) = self.link_stack.pop() {
                    let text = spans_plain_text(self.spans.get(start..).unwrap_or_default());
                    let text = text.trim().to_string();
                    let show_url = !url.is_empty()
                        && !url.starts_with('#')
                        && text != url
                        && text != url.trim_start_matches("mailto:");
                    if let Some(entry) = self.links.get_mut(id as usize) {
                        entry.0 = if text.is_empty() { url.clone() } else { text };
                    }
                    if show_url {
                        let style = InlineStyle {
                            muted: true,
                            url: true,
                            link: Some(id),
                            ..InlineStyle::default()
                        };
                        self.push_span(&format!(" ({url})"), style);
                    }
                    self.style.link = self.link_stack.last().map(|(id, _, _)| *id);
                }
            }
            TagEnd::Image => {
                if let Some((alt, url)) = self.image.take() {
                    let label = if alt.trim().is_empty() {
                        url.rsplit('/').next().unwrap_or(&url).to_string()
                    } else {
                        alt.trim().to_string()
                    };
                    let id = self.links.len() as u32;
                    self.links.push((format!("image: {label}"), url, true));
                    let style = InlineStyle {
                        muted: true,
                        link: self.style.link.or(Some(id)),
                        ..InlineStyle::default()
                    };
                    // Record the image's own position even when nested in a link.
                    if self.style.link.is_some() {
                        self.push_span(
                            "",
                            InlineStyle {
                                link: Some(id),
                                ..InlineStyle::default()
                            },
                        );
                    }
                    self.push_span(&format!("[image: {label}]"), style);
                }
            }
            _ => {}
        }
    }

    fn push_text(&mut self, text: &str) {
        if self.in_code {
            self.code.push_str(text);
            return;
        }
        if self.in_html {
            self.html.push_str(text);
            return;
        }
        if let Some((alt, _)) = self.image.as_mut() {
            alt.push_str(text);
            return;
        }
        self.push_span(text, self.style);
    }

    fn push_span(&mut self, text: &str, style: InlineStyle) {
        if self.target.is_none() {
            self.target = Some(InlineTarget::Paragraph);
        }
        self.spans.push(Span {
            text: text.to_string(),
            style,
        });
    }

    fn flush_inline(&mut self) {
        let target = match self.target {
            Some(InlineTarget::Cell) | None => return,
            Some(target) => target,
        };
        self.target = None;
        let spans = trim_spans(std::mem::take(&mut self.spans));
        if spans.is_empty() {
            return;
        }
        match target {
            InlineTarget::Paragraph => self.push_block(Block::Paragraph(spans)),
            InlineTarget::Heading(level) => self.push_block(Block::Heading { level, spans }),
            InlineTarget::Cell => {}
        }
    }

    fn push_block(&mut self, block: Block) {
        match self.stack.last_mut() {
            Some(Frame::Root(blocks))
            | Some(Frame::Quote(_, blocks))
            | Some(Frame::Item { blocks, .. })
            | Some(Frame::Footnote { blocks, .. }) => blocks.push(block),
            Some(Frame::List { .. }) | None => {}
        }
    }

    fn pop_frame(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        let Some(frame) = self.stack.pop() else {
            return;
        };
        match frame {
            Frame::Root(_) => {}
            Frame::Quote(kind, blocks) => self.push_block(Block::Quote { kind, blocks }),
            Frame::List {
                start,
                tight,
                items,
            } => self.push_block(Block::List {
                ordered_start: start,
                tight,
                items,
            }),
            Frame::Item { task, blocks } => {
                if let Some(Frame::List { items, .. }) = self.stack.last_mut() {
                    items.push(ListItem { task, blocks });
                } else {
                    self.push_block(Block::Quote { kind: None, blocks });
                }
            }
            Frame::Footnote { label, blocks } => self.push_block(Block::Footnote { label, blocks }),
        }
    }
}

fn is_html_comment_only(html: &str) -> bool {
    let mut rest = html.trim();
    while let Some(after) = rest.strip_prefix("<!--") {
        match after.find("-->") {
            Some(end) => rest = after[end + 3..].trim_start(),
            None => return true,
        }
    }
    rest.is_empty()
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Ctx {
    theme: Theme,
    depth: usize,
}

#[derive(Default)]
struct Out {
    lines: Vec<String>,
    headings: Vec<HeadingInfo>,
    links: Vec<(u32, usize)>,
    code: Vec<CodeBlockInfo>,
}

impl Out {
    fn blank(&mut self) {
        self.lines.push(String::new());
    }

    /// Append `child`, prefixing its first line with `first` and the
    /// rest with `rest`. Blank child lines get `blank` instead.
    fn append(&mut self, child: Out, first: &str, rest: &str, blank: &str) {
        let base = self.lines.len();
        for (idx, line) in child.lines.into_iter().enumerate() {
            let prefix = if idx == 0 { first } else { rest };
            if line.is_empty() && idx != 0 {
                self.lines.push(blank.to_string());
            } else {
                let mut text = String::with_capacity(prefix.len() + line.len());
                text.push_str(prefix);
                text.push_str(&line);
                self.lines.push(text);
            }
        }
        self.headings
            .extend(child.headings.into_iter().map(|mut h| {
                h.line += base;
                h
            }));
        self.links
            .extend(child.links.into_iter().map(|(id, line)| (id, line + base)));
        self.code.extend(child.code.into_iter().map(|mut c| {
            c.start_line += base;
            c.end_line += base;
            c
        }));
    }

    fn push_wrapped(&mut self, lines: Vec<Vec<Span>>, indent: &str, theme: Theme) {
        let mut scratch = String::new();
        for line in lines {
            let idx = self.lines.len();
            for span in &line {
                if let Some(id) = span.style.link {
                    self.links.push((id, idx));
                }
            }
            scratch.clear();
            scratch.push_str(indent);
            write_spans_inline(&mut scratch, &line, theme);
            // Exact-size copy: rendered lines live as long as the document.
            self.lines.push(scratch.as_str().to_owned());
        }
    }
}

fn render_blocks(blocks: &[Block], width: usize, ctx: Ctx, tight: bool) -> Out {
    let mut out = Out::default();
    for (idx, block) in blocks.iter().enumerate() {
        if idx > 0 && !tight {
            out.blank();
        }
        render_block(&mut out, block, width, ctx);
    }
    out
}

fn render_block(out: &mut Out, block: &Block, width: usize, ctx: Ctx) {
    let theme = ctx.theme;
    match block {
        Block::Paragraph(spans) => out.push_wrapped(wrap_styled(spans, width.max(8)), "", theme),
        Block::Heading { level, spans } => render_heading(out, *level, spans, width, theme),
        Block::Code { language, text } => {
            let start = out.lines.len();
            render_code_block(&mut out.lines, language.as_deref(), text, width, theme);
            out.code.push(CodeBlockInfo {
                language: language.clone(),
                code: text.clone(),
                start_line: start,
                end_line: out.lines.len().saturating_sub(1),
            });
        }
        Block::Quote { kind, blocks } => {
            let mut child = render_blocks(blocks, width.saturating_sub(2).max(8), ctx, false);
            let (bar, blank) = match kind {
                Some(kind) => {
                    let (label, color) = alert_label(*kind, theme);
                    let mut titled = Out::default();
                    titled
                        .lines
                        .push(format!("\x1b[{color}m\x1b[1m{label}\x1b[0m"));
                    titled.append(child, "", "", "");
                    child = titled;
                    (
                        format!("\x1b[{color}m| \x1b[0m"),
                        format!("\x1b[{color}m|\x1b[0m"),
                    )
                }
                None => (muted("> ", theme), muted(">", theme)),
            };
            out.append(child, &bar, &bar, &blank);
        }
        Block::Rule => out.lines.push(muted(&"-".repeat(width.min(72)), theme)),
        Block::List {
            ordered_start,
            tight,
            items,
        } => render_list(out, *ordered_start, *tight, items, width, ctx),
        Block::Table {
            aligns,
            header,
            rows,
        } => render_table(out, aligns, header, rows, width, theme),
        Block::Footnote { label, blocks } => {
            let marker = format!("[{label}]: ");
            let marker_width = visible_width(&marker);
            let child = render_blocks(
                blocks,
                width.saturating_sub(marker_width).max(8),
                ctx,
                false,
            );
            let child = if child.lines.is_empty() {
                Out {
                    lines: vec![String::new()],
                    ..Out::default()
                }
            } else {
                child
            };
            out.append(
                child,
                &accent(&marker, theme),
                &" ".repeat(marker_width),
                "",
            );
        }
        Block::Html(html) => {
            for raw in html.lines() {
                let wrapped = wrap_code_line(raw, width, width.saturating_sub(2).max(8));
                if wrapped.is_empty() {
                    out.blank();
                }
                for line in wrapped {
                    out.lines.push(muted(&line, theme));
                }
            }
        }
    }
}

fn alert_label(kind: BlockQuoteKind, theme: Theme) -> (&'static str, &'static str) {
    let light = theme == Theme::Light;
    match kind {
        BlockQuoteKind::Note => ("Note", accent_code(theme)),
        BlockQuoteKind::Tip => (
            "Tip",
            if light {
                "38;2;21;128;61"
            } else {
                "38;2;126;231;135"
            },
        ),
        BlockQuoteKind::Important => (
            "Important",
            if light {
                "38;2;109;40;217"
            } else {
                "38;2;196;167;255"
            },
        ),
        BlockQuoteKind::Warning => (
            "Warning",
            if light {
                "38;2;146;64;14"
            } else {
                "38;2;255;209;102"
            },
        ),
        BlockQuoteKind::Caution => (
            "Caution",
            if light {
                "38;2;190;24;24"
            } else {
                "38;2;255;107;107"
            },
        ),
    }
}

fn render_heading(out: &mut Out, level: usize, spans: &[Span], width: usize, theme: Theme) {
    let text: String = spans
        .iter()
        .filter(|span| !span.style.url)
        .map(|span| span.text.as_str())
        .collect::<String>()
        .trim()
        .to_string();
    let prefix = if level >= 3 {
        format!("{} ", "#".repeat(level.min(6)))
    } else {
        String::new()
    };
    let style = InlineStyle {
        strong: true,
        accent: true,
        ..InlineStyle::default()
    };
    let heading_spans: Vec<Span> = spans
        .iter()
        .map(|span| Span {
            text: span.text.clone(),
            style: if span.style.url {
                span.style
            } else {
                InlineStyle {
                    link: span.style.link,
                    ..style
                }
            },
        })
        .collect();
    let avail = width.saturating_sub(prefix.len()).max(8);
    let lines = wrap_styled(&heading_spans, avail);
    out.headings.push(HeadingInfo {
        level,
        text: text.clone(),
        anchor: String::new(),
        line: out.lines.len(),
    });
    let mut widest = 0;
    let mut scratch = String::new();
    for (idx, line) in lines.into_iter().enumerate() {
        widest = widest.max(spans_width(&line));
        scratch.clear();
        if idx == 0 {
            scratch.push_str(&muted(&prefix, theme));
        } else {
            scratch.extend(std::iter::repeat(' ').take(prefix.len()));
        }
        let line_idx = out.lines.len();
        for span in &line {
            if let Some(id) = span.style.link {
                out.links.push((id, line_idx));
            }
        }
        write_spans_inline(&mut scratch, &line, theme);
        // Exact-size copy: rendered lines live as long as the document.
        out.lines.push(scratch.as_str().to_owned());
    }
    if level == 1 {
        out.lines
            .push(accent(&"-".repeat(widest.min(width).max(3)), theme));
    }
}

fn render_list(
    out: &mut Out,
    ordered_start: Option<u64>,
    tight: bool,
    items: &[ListItem],
    width: usize,
    ctx: Ctx,
) {
    let theme = ctx.theme;
    let start = ordered_start.unwrap_or(1);
    let last_number = start + items.len().saturating_sub(1) as u64;
    let number_width = last_number.to_string().len();
    let lead = if ctx.depth == 0 { "  " } else { "" };

    for (idx, item) in items.iter().enumerate() {
        if idx > 0 && !tight {
            out.blank();
        }
        let bullet = match ordered_start {
            Some(_) => format!("{:>number_width$}.", start + idx as u64),
            None => "-".to_string(),
        };
        let (marker_plain, marker_styled) = match item.task {
            Some(true) => {
                let plain = format!("{lead}[x] ");
                let styled = format!("{lead}{}", string_color("[x]", theme)) + " ";
                (plain, styled)
            }
            Some(false) => {
                let plain = format!("{lead}[ ] ");
                (plain.clone(), muted(&plain, theme))
            }
            None => {
                let plain = format!("{lead}{bullet} ");
                (plain.clone(), muted(&plain, theme))
            }
        };
        let marker_width = visible_width(&marker_plain);
        let child_ctx = Ctx {
            theme,
            depth: ctx.depth + 1,
        };
        let mut child = render_blocks(
            &item.blocks,
            width.saturating_sub(marker_width).max(8),
            child_ctx,
            tight,
        );
        if child.lines.is_empty() {
            child.lines.push(String::new());
        }
        out.append(child, &marker_styled, &" ".repeat(marker_width), "");
    }
}

fn render_table<'a>(
    out: &mut Out,
    aligns: &[Alignment],
    header: &'a [Vec<Span>],
    rows: &'a [Vec<Vec<Span>>],
    width: usize,
    theme: Theme,
) {
    let cols = header
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(0));
    if cols == 0 {
        return;
    }
    let cell = |row: &'a [Vec<Span>], col: usize| -> &'a [Span] {
        row.get(col).map(Vec::as_slice).unwrap_or(&[])
    };
    let header_cells: Vec<Vec<Span>> = (0..cols)
        .map(|c| {
            cell(header, c)
                .iter()
                .map(|s| Span {
                    text: s.text.clone(),
                    style: InlineStyle {
                        strong: true,
                        ..s.style
                    },
                })
                .collect()
        })
        .collect();
    let header_refs: Vec<&[Span]> = header_cells.iter().map(Vec::as_slice).collect();
    let body: Vec<Vec<&[Span]>> = rows
        .iter()
        .map(|row| (0..cols).map(|c| cell(row, c)).collect())
        .collect();

    // Natural layout (only hard breaks) of every cell, header first. It is
    // reused when rendering a cell whose natural layout fits its column.
    let natural: Vec<Vec<(Vec<Vec<Span>>, usize)>> = std::iter::once(&header_refs)
        .chain(body.iter())
        .map(|row| {
            row.iter()
                .map(|spans| wrap_styled_measured(spans, usize::MAX / 4))
                .collect()
        })
        .collect();
    let mut widths = vec![1usize; cols];
    for row in &natural {
        for (c, (lines, _)) in row.iter().enumerate() {
            let natural = lines.iter().map(|l| spans_width(l)).max().unwrap_or(0);
            widths[c] = widths[c].max(natural);
        }
    }
    // Shrink to fit: indent(2) + borders/padding (3 per column + 1).
    let overhead = 2 + 3 * cols + 1;
    let available = width.saturating_sub(overhead).max(cols);
    while widths.iter().sum::<usize>() > available {
        let (idx, max_w) = widths
            .iter()
            .enumerate()
            .max_by_key(|(_, w)| **w)
            .map(|(i, w)| (i, *w))
            .unwrap_or((0, 1));
        if max_w <= 3 {
            break;
        }
        widths[idx] -= 1;
    }

    let border = |fill: char| -> String {
        let mut s = String::from("  +");
        for w in &widths {
            s.push_str(&fill.to_string().repeat(w + 2));
            s.push('+');
        }
        muted(&s, theme)
    };

    let render_row = |out: &mut Out, row: &[&[Span]], natural: Vec<(Vec<Vec<Span>>, usize)>| {
        let wrapped: Vec<Vec<Vec<Span>>> = row
            .iter()
            .zip(natural)
            .enumerate()
            .map(|(c, (spans, (lines, layout_width)))| {
                let width = widths[c].max(1);
                if layout_width <= width {
                    lines
                } else {
                    wrap_styled(spans, width)
                }
            })
            .collect();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1).max(1);
        let mut s = String::new();
        for line_idx in 0..height {
            let out_idx = out.lines.len();
            s.clear();
            paint(&mut s, muted_code(theme), "  |");
            for (c, cell_lines) in wrapped.iter().enumerate() {
                let line: &[Span] = cell_lines.get(line_idx).map(Vec::as_slice).unwrap_or(&[]);
                for span in line {
                    if let Some(id) = span.style.link {
                        out.links.push((id, out_idx));
                    }
                }
                let used = spans_width(line);
                let pad = widths[c].saturating_sub(used);
                let (left, right) = match aligns.get(c) {
                    Some(Alignment::Right) => (pad, 0),
                    Some(Alignment::Center) => (pad / 2, pad - pad / 2),
                    _ => (0, pad),
                };
                s.push(' ');
                s.extend(std::iter::repeat(' ').take(left));
                write_spans_inline(&mut s, line, theme);
                s.extend(std::iter::repeat(' ').take(right));
                s.push(' ');
                paint(&mut s, muted_code(theme), "|");
            }
            // Exact-size copy: rendered lines live as long as the document.
            out.lines.push(s.as_str().to_owned());
        }
    };

    let mut natural = natural.into_iter();
    out.lines.push(border('-'));
    let header_natural = natural.next().unwrap_or_default();
    if header.iter().any(|c| !c.is_empty()) {
        render_row(out, &header_refs, header_natural);
        out.lines.push(border('='));
    }
    for (row, row_natural) in body.iter().zip(natural) {
        render_row(out, row, row_natural);
    }
    out.lines.push(border('-'));
}

fn render_code_block(
    lines: &mut Vec<String>,
    language: Option<&str>,
    text: &str,
    width: usize,
    theme: Theme,
) {
    let content_width = width.saturating_sub(6).max(12);
    let continuation_width = content_width.saturating_sub(2).max(10);
    let language = language.filter(|value| !value.is_empty());
    let highlighter = Highlighter::for_language(language);
    let mut scratch = String::new();

    lines.push(code_block_top_border(language, content_width, theme));

    if text.is_empty() {
        lines.push(render_code_row(
            &mut scratch,
            "",
            content_width,
            theme,
            &highlighter,
        ));
        lines.push(code_block_bottom_border(content_width, theme));
        return;
    }

    for raw in text.lines() {
        let raw = if raw.contains('\t') {
            std::borrow::Cow::Owned(raw.replace('\t', "    "))
        } else {
            std::borrow::Cow::Borrowed(raw)
        };
        let wrapped = wrap_code_line(&raw, content_width, continuation_width);
        if wrapped.is_empty() {
            lines.push(render_code_row(
                &mut scratch,
                "",
                content_width,
                theme,
                &highlighter,
            ));
            continue;
        }

        for line in wrapped {
            lines.push(render_code_row(
                &mut scratch,
                &line,
                content_width,
                theme,
                &highlighter,
            ));
        }
    }

    lines.push(code_block_bottom_border(content_width, theme));
}

// ---------------------------------------------------------------------------
// Inline wrapping (whitespace + CJK aware)
// ---------------------------------------------------------------------------

/// A run of word text with a single style: `text[start..end]` of the
/// owning [`Tokens`] buffer. An empty range is a zero-width link marker.
#[derive(Clone, Copy)]
struct Piece {
    style: InlineStyle,
    start: usize,
    end: usize,
}

/// An unbreakable unit: `pieces[pieces.start..pieces.end]` of [`Tokens`].
struct Word {
    pieces: std::ops::Range<usize>,
    space_before: bool,
    width: usize,
}

enum Token {
    Word(Word),
    Break,
}

/// Tokenized inline content. Word text lives in one shared buffer so that
/// tokenizing does not allocate per word (or per CJK character).
#[derive(Default)]
struct Tokens {
    tokens: Vec<Token>,
    pieces: Vec<Piece>,
    text: String,
}

impl Tokens {
    fn new_word(&mut self, space_before: bool) {
        let at = self.pieces.len();
        self.tokens.push(Token::Word(Word {
            pieces: at..at,
            space_before,
            width: 0,
        }));
    }

    fn has_last_word(&self) -> bool {
        matches!(self.tokens.last(), Some(Token::Word(_)))
    }

    /// Append a zero-width marker piece to the last word (never merged).
    fn push_marker(&mut self, style: InlineStyle) {
        let Some(Token::Word(word)) = self.tokens.last_mut() else {
            return;
        };
        let at = self.text.len();
        self.pieces.push(Piece {
            style,
            start: at,
            end: at,
        });
        word.pieces.end = self.pieces.len();
    }

    /// Append text to the last word, merging into its last piece when the
    /// style matches (including into a marker piece).
    fn push_text(&mut self, text: &str, style: InlineStyle) {
        let Some(Token::Word(word)) = self.tokens.last_mut() else {
            return;
        };
        word.width += UnicodeWidthStr::width(text);
        let start = self.text.len();
        self.text.push_str(text);
        let end = self.text.len();
        if word.pieces.end > word.pieces.start {
            // The word's last piece is also the last piece overall, and its
            // text ends at the end of the buffer, so extending it is exact.
            let last = &mut self.pieces[word.pieces.end - 1];
            if last.style == style {
                last.end = end;
                return;
            }
        }
        self.pieces.push(Piece { style, start, end });
        word.pieces.end = self.pieces.len();
    }

    /// Append one character whose `char_width` is `width`. Equivalent to
    /// `push_text` with the one-character string, without the string scan.
    fn push_char(&mut self, ch: char, width: usize, style: InlineStyle) {
        let mut buf = [0u8; 4];
        let text = ch.encode_utf8(&mut buf);
        let Some(Token::Word(word)) = self.tokens.last_mut() else {
            return;
        };
        word.width += single_char_str_width(ch, width);
        let start = self.text.len();
        self.text.push_str(text);
        let end = self.text.len();
        if word.pieces.end > word.pieces.start {
            let last = &mut self.pieces[word.pieces.end - 1];
            if last.style == style {
                last.end = end;
                return;
            }
        }
        self.pieces.push(Piece { style, start, end });
        word.pieces.end = self.pieces.len();
    }

    /// Does the last word end with a character that must not end a line?
    fn last_is_no_end(&self) -> bool {
        match self.tokens.last() {
            Some(Token::Word(w)) if w.pieces.end > w.pieces.start => {
                let piece = self.pieces[w.pieces.end - 1];
                self.text[piece.start..piece.end]
                    .chars()
                    .last()
                    .is_some_and(is_no_line_end)
            }
            _ => false,
        }
    }
}

/// Characters that must not start a line (kinsoku shori + ASCII closers).
fn is_no_line_start(ch: char) -> bool {
    matches!(
        ch,
        '、' | '。'
            | '，'
            | '．'
            | '・'
            | '：'
            | '；'
            | '？'
            | '！'
            | '）'
            | '」'
            | '』'
            | '】'
            | '〕'
            | '〉'
            | '》'
            | '］'
            | '｝'
            | 'ー'
            | '々'
            | 'ぁ'
            | 'ぃ'
            | 'ぅ'
            | 'ぇ'
            | 'ぉ'
            | 'っ'
            | 'ゃ'
            | 'ゅ'
            | 'ょ'
            | 'ァ'
            | 'ィ'
            | 'ゥ'
            | 'ェ'
            | 'ォ'
            | 'ッ'
            | 'ャ'
            | 'ュ'
            | 'ョ'
            | '.'
            | ','
            | ';'
            | ':'
            | '!'
            | '?'
            | ')'
            | ']'
            | '}'
    )
}

/// Characters that must not end a line (opening brackets).
fn is_no_line_end(ch: char) -> bool {
    matches!(
        ch,
        '（' | '「' | '『' | '【' | '〔' | '〈' | '《' | '［' | '｛'
    )
}

fn char_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

/// `UnicodeWidthStr::width` of the one-character string `ch`, given
/// `char_width(ch)`: the two only differ for control characters, which are
/// one column wide in string context (checked exhaustively in tests).
fn single_char_str_width(ch: char, char_width: usize) -> usize {
    if ch.is_control() {
        1
    } else {
        char_width
    }
}

fn tokenize(spans: &[Span]) -> Tokens {
    // Word text never exceeds the input; words and pieces are at most one
    // per character, typically far fewer.
    let len: usize = spans.iter().map(|span| span.text.len()).sum();
    let mut t = Tokens {
        tokens: Vec::with_capacity(len / 4 + 1),
        pieces: Vec::with_capacity(len / 4 + 1),
        text: String::with_capacity(len),
    };
    let mut pending_space = false;
    let mut open = false; // may the next char be glued to the last word?

    for span in spans {
        if span.text.is_empty() {
            // Zero-width marker (used to record link positions).
            if span.style.link.is_some() {
                if !(open && !pending_space) || !t.has_last_word() {
                    t.new_word(pending_space);
                    pending_space = false;
                    open = true;
                }
                t.push_marker(span.style);
            }
            continue;
        }
        if span.style.code {
            if !(open && !pending_space) || !t.has_last_word() {
                t.new_word(pending_space);
            }
            pending_space = false;
            if span.text.contains('\n') {
                t.push_text(&span.text.replace('\n', " "), span.style);
            } else {
                t.push_text(&span.text, span.style);
            }
            open = true;
            continue;
        }
        for ch in span.text.chars() {
            if ch == '\n' {
                t.tokens.push(Token::Break);
                pending_space = false;
                open = false;
                continue;
            }
            if ch.is_whitespace() {
                pending_space = true;
                open = false;
                continue;
            }
            let ch_width = char_width(ch);
            let wide = ch_width >= 2;
            let attach = t.has_last_word()
                && !pending_space
                && ((open && !wide)
                    || is_no_line_start(ch)
                    || (open && wide && t.last_is_no_end()));
            if !attach {
                t.new_word(pending_space);
            }
            pending_space = false;
            t.push_char(ch, ch_width, span.style);
            open = !wide || is_no_line_end(ch);
        }
    }
    t
}

/// Append styled text to a line, merging with the last span when the style
/// matches (empty text still creates a span, which records link positions).
fn push_line_text(line: &mut Vec<Span>, text: &str, style: InlineStyle) {
    if let Some(last) = line.last_mut() {
        if last.style == style {
            last.text.push_str(text);
            return;
        }
    }
    line.push(Span {
        text: text.to_string(),
        style,
    });
}

fn wrap_styled(spans: &[Span], width: usize) -> Vec<Vec<Span>> {
    wrap_styled_impl::<false>(spans, width).0
}

/// `wrap_styled` plus the largest line width used for the wrapping
/// decisions. Wrapping the same spans at any width >= that value gives the
/// same lines (no decision can change).
fn wrap_styled_measured(spans: &[Span], width: usize) -> (Vec<Vec<Span>>, usize) {
    wrap_styled_impl::<true>(spans, width)
}

/// Shared implementation; the measurement is compiled out when `MEASURE`
/// is false (the hot path for paragraphs).
fn wrap_styled_impl<const MEASURE: bool>(spans: &[Span], width: usize) -> (Vec<Vec<Span>>, usize) {
    let width = width.max(1);
    let mut lines: Vec<Vec<Span>> = Vec::new();
    let mut current: Vec<Span> = Vec::new();
    let mut current_width = 0usize;
    let mut layout_width = 0usize;

    let t = tokenize(spans);
    for token in &t.tokens {
        let word = match token {
            Token::Break => {
                lines.push(std::mem::take(&mut current));
                current_width = 0;
                continue;
            }
            Token::Word(word) => word,
        };
        let pieces = &t.pieces[word.pieces.clone()];
        let mut sep = usize::from(word.space_before && current_width > 0);
        if current_width > 0 && current_width + sep + word.width > width {
            lines.push(std::mem::take(&mut current));
            current_width = 0;
            sep = 0;
        }
        if sep == 1 {
            let prev = current.last().map(|s| s.style);
            let next = pieces.first().map(|p| p.style);
            let style = match (prev, next) {
                (Some(a), Some(b)) if a == b => a,
                _ => InlineStyle::default(),
            };
            push_line_text(&mut current, " ", style);
            current_width += 1;
            if MEASURE {
                layout_width = layout_width.max(current_width);
            }
        }
        if word.width <= width.saturating_sub(current_width) {
            current_width += word.width;
            if MEASURE {
                layout_width = layout_width.max(current_width);
            }
            for piece in pieces {
                push_line_text(&mut current, &t.text[piece.start..piece.end], piece.style);
            }
            continue;
        }
        // Word longer than a line: break it by character.
        for piece in pieces {
            let text = &t.text[piece.start..piece.end];
            if text.is_empty() {
                push_line_text(&mut current, text, piece.style);
                continue;
            }
            for (idx, ch) in text.char_indices() {
                let w = char_width(ch);
                if current_width > 0 && current_width + w > width {
                    lines.push(std::mem::take(&mut current));
                    current_width = 0;
                }
                push_line_text(&mut current, &text[idx..idx + ch.len_utf8()], piece.style);
                current_width += w;
                if MEASURE {
                    layout_width = layout_width.max(current_width);
                }
            }
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    while lines.len() > 1 && lines.last().is_some_and(|l| spans_width(l) == 0) {
        lines.pop();
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    (lines, layout_width)
}

fn spans_width(spans: &[Span]) -> usize {
    spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.text.as_str()))
        .sum()
}

fn trim_spans(spans: Vec<Span>) -> Vec<Span> {
    let mut out = spans;
    while out
        .first()
        .is_some_and(|span| span.text.trim().is_empty() && span.style.link.is_none())
    {
        out.remove(0);
    }
    while out
        .last()
        .is_some_and(|span| span.text.trim().is_empty() && span.style.link.is_none())
    {
        out.pop();
    }
    out
}

fn write_spans_inline(out: &mut String, spans: &[Span], theme: Theme) {
    for span in spans {
        write_span(out, span, theme);
    }
}

/// Append `span` as ANSI text: the color in its own SGR sequence (easy to
/// match in tests), then the attributes, the text and a reset.
fn write_span(out: &mut String, span: &Span, theme: Theme) {
    if span.text.is_empty() {
        return;
    }
    let style = span.style;
    let attrs = [
        (style.strong, "1"),
        (style.emphasis, "3"),
        (style.link.is_some() && !style.muted && !style.code, "4"),
        (style.strike, "9"),
    ];
    let color = if style.code {
        Some(match theme {
            Theme::Dark => "38;2;255;160;160",
            Theme::Light => "38;2;153;27;27",
        })
    } else if style.muted {
        Some(muted_code(theme))
    } else if style.link.is_some() || style.accent {
        Some(accent_code(theme))
    } else {
        None
    };
    let any_attr = attrs.iter().any(|(on, _)| *on);
    if color.is_none() && !any_attr {
        out.push_str(&span.text);
        return;
    }
    if let Some(color) = color {
        out.push_str("\x1b[");
        out.push_str(color);
        out.push('m');
    }
    if any_attr {
        out.push_str("\x1b[");
        let mut first = true;
        for (_, code) in attrs.iter().filter(|(on, _)| *on) {
            if !first {
                out.push(';');
            }
            out.push_str(code);
            first = false;
        }
        out.push('m');
    }
    out.push_str(&span.text);
    out.push_str("\x1b[0m");
}

pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&c) {
                            break;
                        }
                    }
                    continue;
                }
                Some(']') => {
                    // OSC: terminated by BEL or ESC \
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                    continue;
                }
                _ => {}
            }
        }
        out.push(ch);
    }
    out
}

fn spans_plain_text(spans: &[Span]) -> String {
    spans.iter().map(|span| span.text.as_str()).collect()
}

/// Display width of a string, ignoring ANSI escape sequences.
pub fn visible_width(input: &str) -> usize {
    if !input.contains('\x1b') {
        // `strip_ansi` only changes text at ESC characters.
        return UnicodeWidthStr::width(input);
    }
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
        let ch_width = char_width(ch);
        if used + ch_width > width {
            break;
        }
        used += ch_width;
        fallback = idx + ch.len_utf8();

        if ch.is_whitespace() || matches!(ch, '/' | '-' | '_' | '&' | '?' | '=') {
            break_at = Some(idx + ch.len_utf8());
        }
    }

    let first_len = input.chars().next().map(char::len_utf8).unwrap_or(1);
    break_at
        .filter(|idx| *idx > 0)
        .unwrap_or(fallback.max(first_len))
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
        "" | "auto" => Ok(detect_theme(std::env::var("COLORFGBG").ok().as_deref())),
        "dark" => Ok(Theme::Dark),
        "light" => Ok(Theme::Light),
        _ => Err(format!("invalid --style={style:?} (use auto|dark|light)")),
    }
}

/// Guess the terminal background from `COLORFGBG` ("fg;bg" or "fg;x;bg").
/// Background colors 7 (white) and 9-15 (bright) are treated as light.
fn detect_theme(colorfgbg: Option<&str>) -> Theme {
    let bg = colorfgbg
        .and_then(|value| value.rsplit(';').next())
        .and_then(|value| value.trim().parse::<u8>().ok());
    match bg {
        Some(7) | Some(9..=15) => Theme::Light,
        _ => Theme::Dark,
    }
}

fn accent_code(theme: Theme) -> &'static str {
    match theme {
        Theme::Dark => "38;2;138;180;248",
        Theme::Light => "38;2;37;99;235",
    }
}

fn muted_code(theme: Theme) -> &'static str {
    match theme {
        Theme::Dark => "38;2;184;184;184",
        Theme::Light => "38;2;80;80;80",
    }
}

fn accent(text: &str, theme: Theme) -> String {
    format!("\x1b[{}m{text}\x1b[0m", accent_code(theme))
}

fn muted(text: &str, theme: Theme) -> String {
    if text.is_empty() {
        return String::new();
    }
    format!("\x1b[{}m{text}\x1b[0m", muted_code(theme))
}

/// Append `\x1b[{sgr}m{text}\x1b[0m`.
fn paint(out: &mut String, sgr: &str, text: &str) {
    out.push_str("\x1b[");
    out.push_str(sgr);
    out.push('m');
    out.push_str(text);
    out.push_str("\x1b[0m");
}

fn push_command(out: &mut String, text: &str, theme: Theme) {
    let color = match theme {
        Theme::Dark => "38;2;255;107;107",
        Theme::Light => "38;2;190;24;24",
    };
    out.push_str("\x1b[");
    out.push_str(color);
    out.push_str("m\x1b[1m");
    out.push_str(text);
    out.push_str("\x1b[0m");
}

fn push_keyword(out: &mut String, text: &str, theme: Theme) {
    let color = match theme {
        Theme::Dark => "38;2;255;107;107",
        Theme::Light => "38;2;190;24;24",
    };
    paint(out, color, text);
}

fn push_flag(out: &mut String, text: &str, theme: Theme) {
    let color = match theme {
        Theme::Dark => "38;2;255;209;102",
        Theme::Light => "38;2;146;64;14",
    };
    paint(out, color, text);
}

fn push_shell_text(out: &mut String, text: &str, theme: Theme) {
    let color = match theme {
        Theme::Dark => "38;2;255;160;160",
        Theme::Light => "38;2;153;27;27",
    };
    paint(out, color, text);
}

fn push_string(out: &mut String, text: &str, theme: Theme) {
    let color = match theme {
        Theme::Dark => "38;2;126;231;135",
        Theme::Light => "38;2;21;128;61",
    };
    paint(out, color, text);
}

fn string_color(text: &str, theme: Theme) -> String {
    let mut out = String::new();
    push_string(&mut out, text, theme);
    out
}

fn push_comment(out: &mut String, text: &str, theme: Theme) {
    let color = match theme {
        Theme::Dark => "38;2;128;139;150",
        Theme::Light => "38;2;106;115;125",
    };
    paint(out, color, text);
}

fn push_code_text(out: &mut String, text: &str, theme: Theme) {
    let color = match theme {
        Theme::Dark => "38;2;230;230;230",
        Theme::Light => "38;2;26;26;26",
    };
    paint(out, color, text);
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

/// Render one code row into the reusable `scratch` buffer and return an
/// exact-size copy (rendered lines are kept for the document's lifetime, so
/// they should not carry spare capacity).
fn render_code_row(
    scratch: &mut String,
    text: &str,
    content_width: usize,
    theme: Theme,
    highlighter: &Highlighter,
) -> String {
    let text = fit_code_cell(text, content_width);
    let padding = content_width.saturating_sub(visible_width(&text));
    scratch.clear();
    paint(scratch, muted_code(theme), "  | ");
    highlighter.write_line(scratch, &text, theme);
    scratch.extend(std::iter::repeat(' ').take(padding));
    paint(scratch, muted_code(theme), " |");
    scratch.as_str().to_owned()
}

fn fit_code_cell(text: &str, width: usize) -> std::borrow::Cow<'_, str> {
    if visible_width(text) <= width {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let ch_width = char_width(ch);
        if used + ch_width > width {
            break;
        }
        used += ch_width;
        out.push(ch);
    }
    std::borrow::Cow::Owned(out)
}

// ---------------------------------------------------------------------------
// Syntax highlighting
// ---------------------------------------------------------------------------

struct LangSpec {
    keywords: &'static [&'static str],
    line_comment: &'static [&'static str],
}

fn lang_spec(lang: &str) -> Option<LangSpec> {
    const RUST: &[&str] = &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
        "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
        "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait",
        "true", "type", "unsafe", "use", "where", "while",
    ];
    const GO: &[&str] = &[
        "break",
        "case",
        "chan",
        "const",
        "continue",
        "default",
        "defer",
        "else",
        "fallthrough",
        "for",
        "func",
        "go",
        "goto",
        "if",
        "import",
        "interface",
        "map",
        "package",
        "range",
        "return",
        "select",
        "struct",
        "switch",
        "type",
        "var",
        "nil",
        "true",
        "false",
    ];
    const PYTHON: &[&str] = &[
        "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
        "elif", "else", "except", "False", "finally", "for", "from", "global", "if", "import",
        "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True",
        "try", "while", "with", "yield",
    ];
    const JS: &[&str] = &[
        "async",
        "await",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "default",
        "delete",
        "do",
        "else",
        "export",
        "extends",
        "false",
        "finally",
        "for",
        "from",
        "function",
        "if",
        "import",
        "in",
        "instanceof",
        "interface",
        "let",
        "new",
        "null",
        "return",
        "super",
        "switch",
        "this",
        "throw",
        "true",
        "try",
        "type",
        "typeof",
        "undefined",
        "var",
        "void",
        "while",
        "yield",
    ];
    const C_LIKE: &[&str] = &[
        "auto",
        "bool",
        "break",
        "case",
        "catch",
        "char",
        "class",
        "const",
        "continue",
        "default",
        "do",
        "double",
        "else",
        "enum",
        "extends",
        "extern",
        "false",
        "final",
        "float",
        "for",
        "fun",
        "if",
        "implements",
        "import",
        "int",
        "interface",
        "long",
        "namespace",
        "new",
        "null",
        "package",
        "private",
        "protected",
        "public",
        "return",
        "short",
        "signed",
        "sizeof",
        "static",
        "struct",
        "switch",
        "template",
        "this",
        "throw",
        "true",
        "try",
        "typedef",
        "union",
        "unsigned",
        "using",
        "val",
        "var",
        "void",
        "volatile",
        "when",
        "while",
        "func",
        "let",
        "guard",
        "nil",
        "self",
        "#include",
        "#define",
    ];
    const RUBY: &[&str] = &[
        "begin", "class", "def", "do", "else", "elsif", "end", "ensure", "false", "if", "module",
        "nil", "require", "rescue", "return", "self", "then", "true", "unless", "until", "when",
        "while", "yield",
    ];
    const SQL: &[&str] = &[
        "SELECT", "FROM", "WHERE", "INSERT", "INTO", "VALUES", "UPDATE", "SET", "DELETE", "CREATE",
        "TABLE", "DROP", "ALTER", "JOIN", "LEFT", "RIGHT", "INNER", "OUTER", "ON", "GROUP", "BY",
        "ORDER", "LIMIT", "AND", "OR", "NOT", "NULL", "AS", "PRIMARY", "KEY", "INDEX", "select",
        "from", "where", "insert", "into", "values", "update", "set", "delete", "create", "table",
        "join", "on", "group", "by", "order", "limit", "and", "or", "not", "null", "as",
    ];
    const DATA: &[&str] = &["true", "false", "null", "yes", "no", "on", "off"];
    let spec = match lang {
        "rust" | "rs" => LangSpec {
            keywords: RUST,
            line_comment: &["//"],
        },
        "go" | "golang" => LangSpec {
            keywords: GO,
            line_comment: &["//"],
        },
        "python" | "py" | "python3" => LangSpec {
            keywords: PYTHON,
            line_comment: &["#"],
        },
        "js" | "javascript" | "jsx" | "ts" | "typescript" | "tsx" | "mjs" | "cjs" => LangSpec {
            keywords: JS,
            line_comment: &["//"],
        },
        "c" | "h" | "cpp" | "c++" | "cc" | "hpp" | "java" | "kotlin" | "kt" | "swift" | "cs"
        | "csharp" | "scala" | "dart" => LangSpec {
            keywords: C_LIKE,
            line_comment: &["//"],
        },
        "ruby" | "rb" => LangSpec {
            keywords: RUBY,
            line_comment: &["#"],
        },
        "sql" => LangSpec {
            keywords: SQL,
            line_comment: &["--"],
        },
        "json" | "jsonc" | "json5" => LangSpec {
            keywords: DATA,
            line_comment: &["//"],
        },
        "yaml" | "yml" | "toml" | "ini" | "conf" | "dockerfile" | "makefile" | "make" => LangSpec {
            keywords: DATA,
            line_comment: &["#"],
        },
        _ => return None,
    };
    Some(spec)
}

/// Syntax highlighter for one code block (chosen once per block).
enum Highlighter {
    Shell,
    /// No language given: lines that look like shell commands are
    /// highlighted as shell, the rest as plain code.
    Auto,
    Diff,
    Generic(LangSpec),
    Plain,
}

impl Highlighter {
    fn for_language(language: Option<&str>) -> Self {
        let lang = language.unwrap_or_default().to_ascii_lowercase();
        if matches!(
            lang.as_str(),
            "bash" | "sh" | "shell" | "zsh" | "console" | "fish" | "powershell" | "ps1"
        ) {
            return Highlighter::Shell;
        }
        if lang.is_empty() {
            return Highlighter::Auto;
        }
        if matches!(lang.as_str(), "diff" | "patch") {
            return Highlighter::Diff;
        }
        match lang_spec(&lang) {
            Some(spec) => Highlighter::Generic(spec),
            None => Highlighter::Plain,
        }
    }

    fn write_line(&self, out: &mut String, text: &str, theme: Theme) {
        match self {
            Highlighter::Shell => highlight_shell_line(out, text, theme),
            Highlighter::Auto if looks_like_shell_line(text) => {
                highlight_shell_line(out, text, theme)
            }
            Highlighter::Diff => highlight_diff_line(out, text, theme),
            Highlighter::Generic(spec) => highlight_generic_line(out, text, spec, theme),
            Highlighter::Auto | Highlighter::Plain => push_code_text(out, text, theme),
        }
    }
}

fn highlight_diff_line(out: &mut String, text: &str, theme: Theme) {
    if text.starts_with("+++") || text.starts_with("---") {
        paint(out, "1", text);
    } else if text.starts_with('+') {
        push_string(out, text, theme);
    } else if text.starts_with('-') {
        push_keyword(out, text, theme);
    } else if text.starts_with("@@") {
        paint(out, accent_code(theme), text);
    } else {
        push_code_text(out, text, theme);
    }
}

fn highlight_generic_line(out: &mut String, text: &str, spec: &LangSpec, theme: Theme) {
    // Byte offsets into `text`; unhighlighted text is always the contiguous
    // run `text[plain..i]`.
    let mut i = 0;
    let mut plain = 0;

    let flush_plain = |out: &mut String, plain: usize, i: usize| {
        if plain < i {
            push_code_text(out, &text[plain..i], theme);
        }
    };

    while let Some(ch) = text[i..].chars().next() {
        let rest = &text[i..];
        if spec
            .line_comment
            .iter()
            .any(|prefix| rest.starts_with(prefix))
            || rest.starts_with("/*")
        {
            // `#` only starts a comment at a token boundary (e.g. not `a#b`).
            let boundary = text[..i]
                .chars()
                .next_back()
                .map_or(true, |prev| !prev.is_alphanumeric());
            if boundary {
                flush_plain(out, plain, i);
                push_comment(out, rest, theme);
                return;
            }
        }
        if ch == '"' || ch == '\'' || ch == '`' {
            // Rust lifetimes / char-like apostrophes: only treat as a string
            // when the quote is closed on this line.
            let body = i + ch.len_utf8();
            let mut escaped = false;
            let close = text[body..].char_indices().find_map(|(off, c)| {
                let hit = !escaped && c == ch;
                escaped = !escaped && c == '\\';
                hit.then_some(body + off)
            });
            if let Some(end) = close {
                flush_plain(out, plain, i);
                let end = end + ch.len_utf8();
                push_string(out, &text[i..end], theme);
                i = end;
                plain = i;
                continue;
            }
        }
        let is_word_char = |c: char| c.is_alphanumeric() || c == '_' || c == '#';
        if is_word_char(ch) {
            let start = i;
            i = text[start..]
                .char_indices()
                .find(|(_, c)| !is_word_char(*c))
                .map_or(text.len(), |(off, _)| start + off);
            let word = &text[start..i];
            if spec.keywords.contains(&word) {
                flush_plain(out, plain, start);
                push_keyword(out, word, theme);
                plain = i;
            } else if word.starts_with(|c: char| c.is_ascii_digit()) {
                flush_plain(out, plain, start);
                push_flag(out, word, theme);
                plain = i;
            }
            continue;
        }
        i += ch.len_utf8();
    }
    flush_plain(out, plain, i);
}

fn highlight_shell_line(out: &mut String, text: &str, theme: Theme) {
    let trimmed = text.trim_start();
    if trimmed.starts_with('#') {
        push_comment(out, text, theme);
        return;
    }

    let mut token_start = !text.starts_with("  ");
    for token in shell_tokens_preserving_space(text) {
        if token.trim().is_empty() {
            out.push_str(token);
            continue;
        }
        if token_start {
            if token == "$" || token == ">" {
                push_comment(out, token, theme);
                continue;
            }
            push_command(out, token, theme);
            token_start = false;
        } else if matches!(token, "|" | "&&" | "||" | ";") {
            push_comment(out, token, theme);
            token_start = true;
        } else if token.starts_with('-') {
            push_flag(out, token, theme);
        } else if token.starts_with('"') || token.starts_with('\'') {
            push_string(out, token, theme);
        } else if token.contains("://") {
            paint(out, accent_code(theme), token);
        } else {
            push_shell_text(out, token, theme);
        }
    }
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
        "$" | "cat"
            | "cd"
            | "chmod"
            | "cp"
            | "curl"
            | "docker"
            | "echo"
            | "export"
            | "git"
            | "go"
            | "cargo"
            | "make"
            | "mkdir"
            | "mv"
            | "npm"
            | "npx"
            | "pip"
            | "pnpm"
            | "python"
            | "rm"
            | "rustup"
            | "sh"
            | "sudo"
            | "tar"
            | "unzip"
            | "wget"
            | "yarn"
    ) || command.contains('=')
        || command.starts_with("./")
}

/// Split `text` into maximal runs of whitespace / non-whitespace.
fn shell_tokens_preserving_space(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        let first = rest.chars().next()?;
        let is_space = first.is_whitespace();
        let end = rest
            .char_indices()
            .find(|(_, c)| c.is_whitespace() != is_space)
            .map_or(rest.len(), |(idx, _)| idx);
        let (token, tail) = rest.split_at(end);
        rest = tail;
        Some(token)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(md: &str, width: usize) -> String {
        render_markdown(
            md,
            RenderOptions {
                style: "dark".to_string(),
                width,
            },
        )
        .unwrap()
    }

    fn plain(md: &str, width: usize) -> String {
        strip_ansi(&render(md, width))
    }

    fn doc(md: &str, width: usize) -> Rendered {
        render_document(
            md,
            &RenderOptions {
                style: "dark".to_string(),
                width,
            },
        )
        .unwrap()
    }

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
        let p = plain(
            "- one\n- **two**\n\n> quote\n\n```rust\nfn main() {}\n```\n",
            50,
        );
        assert!(p.contains("- one"));
        assert!(p.contains("> quote"));
        assert!(p.contains("rust"));
        assert!(p.contains("fn main() {}"));
    }

    #[test]
    fn render_code_block_wraps_without_truncating() {
        let p = plain(
            "```bash\ncurl -fL -o asset.tar.gz https://example.com/releases/download/v0.1.0/asset.tar.gz\n```\n",
            48,
        );
        assert!(p.contains("bash"));
        assert!(p.contains("curl -fL -o asset.tar.gz"));
        assert!(p.contains("https://"));
        assert!(p.contains("example.com/releases/download/v0.1.0/"));
        assert!(p.contains("asset.tar.gz"));
        assert!(!p.contains("..."));
    }

    #[test]
    fn shell_code_block_highlights_command_in_red() {
        let out = render("```bash\ncurl -fL https://example.com\n```\n", 48);
        assert!(out.contains("\x1b[38;2;255;107;107m\x1b[1mcurl\x1b[0m"));
    }

    #[test]
    fn shell_code_block_colors_plain_arguments_red_too() {
        let out = render("```bash\nmake build\n```\n", 48);
        assert!(out.contains("\x1b[38;2;255;160;160mbuild\x1b[0m"));
    }

    #[test]
    fn unlabeled_shell_like_code_block_is_highlighted() {
        let out = render("```\nmake build\n```\n", 48);
        assert!(out.contains("\x1b[38;2;255;107;107m\x1b[1mmake\x1b[0m"));
    }

    #[test]
    fn shell_continuation_line_is_not_colored_as_command() {
        let out = render(
            "```bash\ncurl https://example.com/releases/download/v0.1.0/asset.tar.gz\n```\n",
            40,
        );
        assert!(!out.contains("\x1b[38;2;255;107;107m\x1b[1mdownload"));
    }

    #[test]
    fn nested_lists_keep_all_items() {
        let p = plain("- outer one\n  - inner a\n  - inner b\n- outer two\n", 60);
        let lines: Vec<&str> = p.lines().collect();
        assert_eq!(
            lines,
            vec![
                "  - outer one",
                "    - inner a",
                "    - inner b",
                "  - outer two"
            ]
        );
    }

    #[test]
    fn loose_list_items_are_separated() {
        let p = plain("- a\n\n- b\n", 60);
        assert_eq!(p, "  - a\n\n  - b\n");
    }

    #[test]
    fn ordered_list_numbers_align() {
        let md: String = (1..=10).map(|n| format!("{n}. item\n")).collect();
        let p = plain(&md, 60);
        assert!(p.contains("   1. item"));
        assert!(p.contains("  10. item"));
    }

    #[test]
    fn tables_render_as_grid() {
        let p = plain(
            "| Name | Value |\n|------|------:|\n| alpha | 1 |\n| beta | 22 |\n",
            60,
        );
        let lines: Vec<&str> = p.lines().collect();
        assert_eq!(lines[0], "  +-------+-------+");
        assert_eq!(lines[1], "  | Name  | Value |");
        assert_eq!(lines[2], "  +=======+=======+");
        assert_eq!(lines[3], "  | alpha |     1 |");
        assert_eq!(lines[4], "  | beta  |    22 |");
        assert_eq!(lines[5], "  +-------+-------+");
    }

    #[test]
    fn wide_tables_wrap_cells_to_fit() {
        let md = "| a | b |\n|---|---|\n| one two three four five six seven | x |\n";
        let p = plain(md, 24);
        for line in p.lines() {
            assert!(UnicodeWidthStr::width(line) <= 24, "too wide: {line:?}");
        }
        assert!(p.contains("seven"));
    }

    #[test]
    fn links_show_url_and_are_recorded() {
        let d = doc(
            "See [docs](https://example.com/docs) and <https://x.y>.\n",
            80,
        );
        let p = strip_ansi(&d.text());
        assert!(p.contains("See docs (https://example.com/docs) and https://x.y."));
        assert_eq!(d.links.len(), 2);
        assert_eq!(d.links[0].text, "docs");
        assert_eq!(d.links[0].url, "https://example.com/docs");
        assert_eq!(d.links[0].line, 0);
    }

    #[test]
    fn anchor_links_hide_url_but_are_recorded() {
        let d = doc("# Top\n\nGo to [top](#top).\n", 80);
        let p = strip_ansi(&d.text());
        assert!(p.contains("Go to top."));
        assert_eq!(d.links[0].url, "#top");
    }

    #[test]
    fn images_show_alt_text() {
        let d = doc("![Logo](img/logo.png)\n", 80);
        assert!(strip_ansi(&d.text()).contains("[image: Logo]"));
        assert!(d.links[0].image);
        assert_eq!(d.links[0].url, "img/logo.png");
    }

    #[test]
    fn task_lists_keep_checkbox() {
        let p = plain("- [x] done\n- [ ] open\n", 60);
        assert!(p.contains("[x] done"));
        assert!(p.contains("[ ] open"));
    }

    #[test]
    fn strikethrough_and_italic_are_styled() {
        let out = render("~~gone~~ *it*\n", 60);
        assert!(out.contains("\x1b[9mgone"));
        assert!(out.contains("\x1b[3mit"));
    }

    #[test]
    fn quotes_contain_nested_blocks() {
        let p = plain("> line\n>\n> - item\n", 60);
        assert_eq!(p, "> line\n>\n>   - item\n");
    }

    #[test]
    fn gfm_alerts_show_title() {
        let p = plain("> [!WARNING]\n> Be careful.\n", 60);
        assert_eq!(p, "| Warning\n| Be careful.\n");
    }

    #[test]
    fn link_in_heading_keeps_url_out_of_toc_text() {
        let d = doc("## [Guide](https://g.example)\n", 60);
        assert_eq!(d.headings[0].text, "Guide");
        assert_eq!(d.headings[0].anchor, "guide");
        assert!(strip_ansi(&d.lines[0]).contains("Guide (https://g.example)"));
    }

    #[test]
    fn footnotes_show_label() {
        let p = plain("Text[^1].\n\n[^1]: The note.\n", 60);
        assert!(p.contains("Text[1]."));
        assert!(p.contains("[1]: The note."));
    }

    #[test]
    fn html_comments_are_hidden() {
        let p = plain("<!-- hidden -->\n\nVisible\n", 60);
        assert!(!p.contains("hidden"));
        assert!(p.contains("Visible"));
    }

    #[test]
    fn cjk_text_wraps_by_width() {
        let md =
            "これは非常に長い日本語の段落です。スペースが含まれていないため、折り返しが必要です。";
        let p = plain(md, 24);
        let lines: Vec<&str> = p.lines().collect();
        assert!(lines.len() > 1);
        for line in &lines {
            assert!(UnicodeWidthStr::width(*line) <= 24, "too wide: {line:?}");
            assert!(!line.starts_with('。') && !line.starts_with('、'));
        }
        assert_eq!(lines.concat(), md);
    }

    #[test]
    fn long_words_are_broken() {
        let p = plain(&"x".repeat(100), 30);
        assert!(p.lines().all(|l| l.len() <= 30));
        assert_eq!(p.replace('\n', ""), "x".repeat(100));
    }

    #[test]
    fn punctuation_after_code_and_refs_stays_attached() {
        let p = plain("Use `md`, then *run*.\n", 60);
        assert!(p.contains("Use md, then run."));
    }

    #[test]
    fn headings_metadata_includes_setext_and_inline_markup() {
        let d = doc(
            "Title\n=====\n\n## Use `md` now\n\nbody\n\n## Use `md` now\n",
            60,
        );
        let texts: Vec<_> = d.headings.iter().map(|h| h.text.as_str()).collect();
        assert_eq!(texts, vec!["Title", "Use md now", "Use md now"]);
        assert_eq!(d.headings[0].level, 1);
        assert_eq!(d.headings[1].anchor, "use-md-now");
        assert_eq!(d.headings[2].anchor, "use-md-now-1");
        for h in &d.headings {
            assert!(strip_ansi(&d.lines[h.line]).contains(&h.text));
        }
    }

    #[test]
    fn section_range_stops_at_same_level() {
        let d = doc("# A\n\n## B\n\nb\n\n### C\n\nc\n\n## D\n\nd\n", 60);
        let b = d.find_heading("b").unwrap();
        let (start, end) = d.section_range(b).unwrap();
        let section: Vec<String> = d.lines[start..end].iter().map(|l| strip_ansi(l)).collect();
        assert_eq!(section.first().unwrap(), "B");
        assert!(section.iter().any(|l| l == "c"));
        assert!(!section.iter().any(|l| l == "D"));
    }

    #[test]
    fn find_heading_by_slug_and_text() {
        let d = doc("# Getting Started\n\n## Install (macOS)\n", 60);
        assert_eq!(d.find_heading("getting-started"), Some(0));
        assert_eq!(d.find_heading("#install-macos"), Some(1));
        assert_eq!(d.find_heading("install"), Some(1));
        assert_eq!(d.find_heading("nope"), None);
    }

    #[test]
    fn code_blocks_are_recorded() {
        let d = doc("text\n\n```sh\nmake build\nmake test\n```\n", 60);
        assert_eq!(d.code_blocks.len(), 1);
        let cb = &d.code_blocks[0];
        assert_eq!(cb.code, "make build\nmake test");
        assert_eq!(cb.language.as_deref(), Some("sh"));
        assert!(strip_ansi(&d.lines[cb.start_line]).contains("+ sh "));
        assert!(strip_ansi(&d.lines[cb.end_line])
            .trim_start()
            .starts_with('+'));
    }

    #[test]
    fn generic_highlighting_colors_keywords_strings_and_comments() {
        let out = render("```python\ndef f(): return \"x\"  # note\n```\n", 60);
        assert!(out.contains("\x1b[38;2;255;107;107mdef\x1b[0m"));
        assert!(out.contains("\x1b[38;2;126;231;135m\"x\"\x1b[0m"));
        assert!(out.contains("\x1b[38;2;128;139;150m# note"));
    }

    #[test]
    fn detect_theme_from_colorfgbg() {
        assert_eq!(detect_theme(Some("0;15")), Theme::Light);
        assert_eq!(detect_theme(Some("15;0")), Theme::Dark);
        assert_eq!(detect_theme(Some("0;default;7")), Theme::Light);
        assert_eq!(detect_theme(None), Theme::Dark);
    }

    #[test]
    fn strip_ansi_handles_osc_and_unicode() {
        assert_eq!(strip_ansi("\x1b[1m日本\x1b[0m"), "日本");
        assert_eq!(strip_ansi("a\x1b]52;c;Zm9v\x07b"), "ab");
    }

    #[test]
    fn control_characters_are_neutralized() {
        let out = render(
            "evil \x1b]52;c;Zm9v\x07 text\n\n```\n\x1b[2Jcode\n```\n",
            60,
        );
        assert!(!out.contains("\x1b]52"));
        assert!(!out.contains("\x1b[2J"));
        let p = strip_ansi(&out);
        assert!(p.contains("evil ^[]52;c;Zm9v^G text"));
        assert!(p.contains("^[[2Jcode"));
        assert_eq!(sanitize_control("a\tb\u{9b}"), "a\tb<U+009B>");
    }

    #[test]
    fn sanitize_fast_path_matches_char_predicate_for_every_char() {
        for ch in (0..=0x10FFFFu32).filter_map(char::from_u32) {
            let bad = (ch.is_control() && !matches!(ch, '\n' | '\t' | '\r'))
                || ('\u{80}'..='\u{9f}').contains(&ch);
            let text = format!("a{ch}b");
            let borrowed = matches!(sanitize_control(&text), std::borrow::Cow::Borrowed(_));
            assert_eq!(borrowed, !bad, "U+{:04X}", ch as u32);
        }
    }

    #[test]
    fn single_char_width_matches_str_width_for_every_char() {
        for ch in (0..=0x10FFFFu32).filter_map(char::from_u32) {
            let mut buf = [0u8; 4];
            let text = ch.encode_utf8(&mut buf);
            assert_eq!(
                single_char_str_width(ch, char_width(ch)),
                UnicodeWidthStr::width(&*text),
                "U+{:04X}",
                ch as u32
            );
        }
    }

    #[test]
    fn slugify_matches_github() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("Install (macOS / Linux)"), "install-macos--linux");
        assert_eq!(slugify("日本語 見出し"), "日本語-見出し");
    }
}
