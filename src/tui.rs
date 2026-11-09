//! Interactive pager (TUI).
//!
//! Features: scrolling, search with in-line highlighting, outline folding,
//! table of contents, link list (follow anchors / relative Markdown files /
//! open URLs), multi-document navigation with history, live reload, and
//! copying code blocks / URLs to the clipboard via OSC 52.

use std::cmp::{max, min};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};
use crossterm::style::{
    Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, queue};
use unicode_width::UnicodeWidthChar;

use crate::input::is_markdown_path;
use crate::render::{
    render_document, sanitize_control, strip_ansi, visible_width, RenderOptions, Rendered,
};

/// A document the pager can show.
#[derive(Clone, Debug)]
pub enum DocSource {
    Path(PathBuf),
    Text { title: String, markdown: String },
}

impl DocSource {
    pub fn label(&self) -> String {
        match self {
            DocSource::Path(path) => display_path(path),
            DocSource::Text { title, .. } => title.clone(),
        }
    }
}

/// Pager configuration.
#[derive(Clone, Debug)]
pub struct ViewConfig {
    pub render: RenderOptions,
    /// Heading (text or anchor) to jump to on start.
    pub section: Option<String>,
    /// Open the file picker on start (e.g. when a directory was given).
    pub show_files: bool,
}

#[derive(Clone, Debug)]
struct Doc {
    title: String,
    path: Option<PathBuf>,
    markdown: String,
    mtime: Option<SystemTime>,
}

#[derive(Clone, Debug)]
struct HistoryEntry {
    doc: Doc,
    entry: Option<usize>,
    line: usize,
}

#[derive(Clone, Debug)]
enum DisplayIndex {
    Identity(usize),
    List(Vec<usize>),
}

impl DisplayIndex {
    fn len(&self) -> usize {
        match self {
            DisplayIndex::Identity(size) => *size,
            DisplayIndex::List(lines) => lines.len(),
        }
    }

    fn at(&self, row: usize) -> usize {
        match self {
            DisplayIndex::Identity(size) => min(row, size.saturating_sub(1)),
            DisplayIndex::List(lines) => {
                if lines.is_empty() {
                    0
                } else {
                    lines[min(row, lines.len() - 1)]
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Overlay {
    None,
    Help,
    Toc,
    Links,
    Files,
}

#[derive(Debug, Default)]
struct Picker {
    idx: usize,
    filter: String,
    draft: String,
    filter_mode: bool,
}

#[derive(Clone, Debug)]
struct PickItem {
    label: String,
    detail: String,
    indent: usize,
    key: usize,
}

#[derive(Debug)]
struct Model {
    entries: Vec<DocSource>,
    entry: Option<usize>,
    doc: Doc,
    history: Vec<HistoryEntry>,
    render_opts: RenderOptions,
    rendered: Rendered,
    plain: Vec<String>,
    offset: usize,
    fold_level: usize,
    display: DisplayIndex,
    width: usize,
    height: usize,
    overlay: Overlay,
    picker_kind: Overlay,
    picker: Picker,
    search_mode: bool,
    search_saved_query: String,
    search_draft: String,
    search_query: String,
    search_matches: Vec<usize>,
    search_idx: usize,
    search_current_line: Option<usize>,
    status_message: String,
    status_until: Option<Instant>,
    last_copied_block: Option<usize>,
    last_reload_check: Instant,
    dirty: bool,
}

/// Backwards-compatible entry point: view a single in-memory document.
pub fn view_markdown(title: &str, markdown: &str, opts: RenderOptions) -> Result<(), String> {
    view_documents(
        vec![DocSource::Text {
            title: title.to_string(),
            markdown: markdown.to_string(),
        }],
        ViewConfig {
            render: opts,
            section: None,
            show_files: false,
        },
    )
}

/// Open the pager on one or more documents (the first one is shown).
pub fn view_documents(sources: Vec<DocSource>, cfg: ViewConfig) -> Result<(), String> {
    let first = sources
        .first()
        .ok_or_else(|| "no documents to show".to_string())?;
    let doc = load_source(first)?;
    let mut model = Model::new(sources, doc, cfg.render.clone());

    let mut stdout = io::stdout();
    terminal::enable_raw_mode().map_err(|err| format!("enable raw mode: {err}"))?;
    if let Err(err) = execute!(stdout, EnterAlternateScreen, Hide) {
        let _ = terminal::disable_raw_mode();
        return Err(format!("enter alternate screen: {err}"));
    }

    let result = (|| {
        model.resize()?;
        model.rerender()?;
        if let Some(section) = cfg.section.as_deref() {
            if !model.jump_to_heading_query(section) {
                model.set_status(&format!("Section not found: {section}"));
            }
        }
        if cfg.show_files && model.entries.len() > 1 {
            model.open_overlay(Overlay::Files);
        } else if model.entries.len() > 1 {
            model.set_status(&format!(
                "{} documents: Tab/Shift-Tab next/prev, F to pick",
                model.entries.len()
            ));
        }
        run_loop(&mut stdout, &mut model)
    })();

    let _ = execute!(stdout, Show, LeaveAlternateScreen, ResetColor);
    let _ = terminal::disable_raw_mode();
    result
}

fn run_loop(stdout: &mut io::Stdout, model: &mut Model) -> Result<(), String> {
    loop {
        if model.dirty {
            model.draw(stdout)?;
            model.dirty = false;
        }
        if event::poll(Duration::from_millis(150)).map_err(|err| format!("poll input: {err}"))? {
            match event::read().map_err(|err| format!("read input: {err}"))? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    model.dirty = true;
                    if model.handle_key(key)? {
                        break;
                    }
                }
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        model.scroll_by(-3);
                        model.dirty = true;
                    }
                    MouseEventKind::ScrollDown => {
                        model.scroll_by(3);
                        model.dirty = true;
                    }
                    _ => {}
                },
                Event::Resize(_, _) => {
                    let anchor = model.anchor_line();
                    model.resize()?;
                    model.rerender()?;
                    model.restore_anchor(anchor);
                    model.dirty = true;
                }
                _ => {}
            }
        }
        if model
            .status_until
            .is_some_and(|until| Instant::now() >= until)
        {
            model.status_message.clear();
            model.status_until = None;
            model.dirty = true;
        }
        if model.last_reload_check.elapsed() >= Duration::from_millis(1000) {
            model.last_reload_check = Instant::now();
            model.reload_if_changed()?;
        }
    }
    Ok(())
}

impl Model {
    fn new(entries: Vec<DocSource>, doc: Doc, render_opts: RenderOptions) -> Self {
        Self {
            entry: if entries.is_empty() { None } else { Some(0) },
            entries,
            doc,
            history: Vec::new(),
            render_opts,
            rendered: Rendered::default(),
            plain: Vec::new(),
            offset: 0,
            fold_level: 0,
            display: DisplayIndex::Identity(0),
            width: 80,
            height: 24,
            overlay: Overlay::None,
            picker_kind: Overlay::None,
            picker: Picker::default(),
            search_mode: false,
            search_saved_query: String::new(),
            search_draft: String::new(),
            search_query: String::new(),
            search_matches: Vec::new(),
            search_idx: 0,
            search_current_line: None,
            status_message: String::new(),
            status_until: None,
            last_copied_block: None,
            last_reload_check: Instant::now(),
            dirty: true,
        }
    }

    fn resize(&mut self) -> Result<(), String> {
        let (w, h) = terminal::size().map_err(|err| format!("terminal size: {err}"))?;
        self.width = usize::from(w);
        self.height = usize::from(h);
        Ok(())
    }

    fn rerender(&mut self) -> Result<(), String> {
        let width = if self.render_opts.width == 0 {
            self.body_text_width()
        } else {
            self.render_opts.width
        };
        self.rendered = render_document(
            &self.doc.markdown,
            &RenderOptions {
                style: self.render_opts.style.clone(),
                width,
            },
        )?;
        self.plain = self.rendered.lines.iter().map(|l| strip_ansi(l)).collect();
        self.last_copied_block = None;
        self.rebuild_display();
        if !self.search_query.is_empty() {
            self.recompute_search();
        }
        self.clamp_offset();
        self.dirty = true;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Key handling
    // ------------------------------------------------------------------

    fn handle_key(&mut self, key: KeyEvent) -> Result<bool, String> {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(true);
        }
        if self.search_mode {
            self.handle_search_key(key);
            self.clamp_offset();
            return Ok(false);
        }
        if self.overlay != Overlay::None {
            self.handle_overlay_key(key)?;
            self.clamp_offset();
            return Ok(false);
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
            KeyCode::Char('?') => self.overlay = Overlay::Help,
            KeyCode::Char('t') => self.open_overlay(Overlay::Toc),
            KeyCode::Char('l') => {
                if self.rendered.links.is_empty() {
                    self.set_status("No links in this document");
                } else {
                    self.open_overlay(Overlay::Links);
                }
            }
            KeyCode::Char('F') => {
                if self.entries.len() > 1 {
                    self.open_overlay(Overlay::Files);
                } else {
                    self.set_status("Only one document (pass several files or a directory)");
                }
            }
            KeyCode::Tab => self.cycle_entry(1)?,
            KeyCode::BackTab => self.cycle_entry(-1)?,
            KeyCode::Backspace => self.go_back()?,
            KeyCode::Char('r') => self.reload(true)?,
            KeyCode::Char('y') => self.copy_visible_code_block(),
            KeyCode::Char('/') => {
                self.search_mode = true;
                self.search_saved_query = self.search_query.clone();
                self.search_draft = self.search_query.clone();
                self.set_search_query_no_jump(&self.search_draft.clone());
            }
            KeyCode::Char('0') => {
                self.fold_level = 0;
                self.rebuild_display();
                self.set_status("Outline: off (press 1-6 to fold)");
            }
            KeyCode::Char(ch @ '1'..='6') => {
                self.fold_level = ch.to_digit(10).unwrap_or(0) as usize;
                self.rebuild_display();
                self.set_status(&format!(
                    "Outline: H{} (press 0 to show all)",
                    self.fold_level
                ));
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Enter => self.scroll_by(1),
            KeyCode::Up | KeyCode::Char('k') => self.scroll_by(-1),
            KeyCode::Char('d') => self.scroll_by(self.page_size() as isize / 2),
            KeyCode::Char('u') => self.scroll_by(-(self.page_size() as isize / 2)),
            KeyCode::PageDown | KeyCode::Char('f') | KeyCode::Char(' ') => {
                self.scroll_by(self.page_size() as isize)
            }
            KeyCode::PageUp | KeyCode::Char('b') => self.scroll_by(-(self.page_size() as isize)),
            KeyCode::Home | KeyCode::Char('g') => self.offset = 0,
            KeyCode::End | KeyCode::Char('G') => self.offset = self.max_offset(),
            KeyCode::Char(']') => self.jump_heading(1),
            KeyCode::Char('[') => self.jump_heading(-1),
            KeyCode::Char('n') => self.jump_next_match(1),
            KeyCode::Char('N') => self.jump_next_match(-1),
            KeyCode::Char('c') => self.set_search_query(""),
            _ => {}
        }
        self.clamp_offset();
        Ok(false)
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.search_mode = false;
                self.search_draft = self.search_saved_query.clone();
                self.set_search_query_no_jump(&self.search_saved_query.clone());
            }
            KeyCode::Enter => {
                self.search_mode = false;
                self.set_search_query(&self.search_draft.clone());
                if self.search_matches.is_empty() && !self.search_query.is_empty() {
                    self.set_status(&format!("Not found: {}", self.search_query));
                }
            }
            KeyCode::Backspace => {
                self.search_draft.pop();
                self.set_search_query_no_jump(&self.search_draft.clone());
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search_draft.clear();
                self.set_search_query_no_jump("");
            }
            KeyCode::Char(ch) => {
                self.search_draft.push(ch);
                let draft = self.search_draft.clone();
                self.set_search_query_no_jump(&draft);
                // Keep the user's draft verbatim (including trailing spaces).
                self.search_draft = draft;
                // Incremental search: bring the first match into view.
                if let Some(line) = self.search_current_line {
                    if !self.is_rendered_line_visible(line) {
                        self.set_offset_for_rendered_line(line);
                    }
                }
            }
            _ => {}
        }
    }

    fn open_overlay(&mut self, kind: Overlay) {
        if self.picker_kind != kind {
            self.picker = Picker::default();
            self.picker_kind = kind;
        }
        self.picker.filter_mode = false;
        self.picker.draft = self.picker.filter.clone();
        self.overlay = kind;
        // Preselect something useful.
        let items = self.picker_items();
        let want = match kind {
            Overlay::Toc => self.current_heading_index(),
            Overlay::Files => self.entry,
            Overlay::Links => {
                let top = self.anchor_line();
                self.rendered.links.iter().position(|l| l.line >= top)
            }
            _ => None,
        };
        if let Some(want) = want {
            if let Some(pos) = items.iter().position(|item| item.key == want) {
                self.picker.idx = pos;
            }
        }
        self.picker.idx = min(self.picker.idx, items.len().saturating_sub(1));
    }

    fn handle_overlay_key(&mut self, key: KeyEvent) -> Result<(), String> {
        if self.overlay == Overlay::Help {
            if matches!(
                key.code,
                KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter
            ) {
                self.overlay = Overlay::None;
            }
            return Ok(());
        }
        if self.picker.filter_mode {
            match key.code {
                KeyCode::Esc => {
                    self.picker.filter_mode = false;
                    self.picker.draft = self.picker.filter.clone();
                }
                KeyCode::Enter => {
                    self.picker.filter_mode = false;
                    self.picker.filter = self.picker.draft.trim().to_string();
                    self.picker.idx = 0;
                }
                KeyCode::Backspace => {
                    self.picker.draft.pop();
                    self.picker.idx = 0;
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.picker.draft.clear();
                    self.picker.idx = 0;
                }
                KeyCode::Char(ch) => {
                    self.picker.draft.push(ch);
                    self.picker.idx = 0;
                }
                _ => {}
            }
        } else {
            let page = self.picker_body_height();
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.overlay = Overlay::None,
                KeyCode::Char('t') if self.overlay == Overlay::Toc => self.overlay = Overlay::None,
                KeyCode::Char('l') if self.overlay == Overlay::Links => {
                    self.overlay = Overlay::None
                }
                KeyCode::Char('F') if self.overlay == Overlay::Files => {
                    self.overlay = Overlay::None
                }
                KeyCode::Char('/') => {
                    self.picker.filter_mode = true;
                    self.picker.draft = self.picker.filter.clone();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.picker.idx = self.picker.idx.saturating_add(1)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.picker.idx = self.picker.idx.saturating_sub(1)
                }
                KeyCode::PageDown => self.picker.idx = self.picker.idx.saturating_add(page),
                KeyCode::PageUp => self.picker.idx = self.picker.idx.saturating_sub(page),
                KeyCode::Home | KeyCode::Char('g') => self.picker.idx = 0,
                KeyCode::End | KeyCode::Char('G') => self.picker.idx = usize::MAX,
                KeyCode::Char('y') if self.overlay == Overlay::Links => {
                    if let Some(item) = self.selected_item() {
                        let url = self.rendered.links[item.key].url.clone();
                        self.copy_to_clipboard(&url, &format!("Copied URL: {url}"));
                    }
                }
                KeyCode::Enter => {
                    if let Some(item) = self.selected_item() {
                        let kind = self.overlay;
                        self.overlay = Overlay::None;
                        self.activate(kind, item.key)?;
                    }
                }
                _ => {}
            }
        }
        let len = self.picker_items().len();
        self.picker.idx = min(self.picker.idx, len.saturating_sub(1));
        Ok(())
    }

    fn activate(&mut self, kind: Overlay, key: usize) -> Result<(), String> {
        match kind {
            Overlay::Toc => {
                if let Some(h) = self.rendered.headings.get(key) {
                    let line = h.line;
                    self.set_offset_for_rendered_line(line);
                }
            }
            Overlay::Links => {
                if let Some(link) = self.rendered.links.get(key) {
                    let url = link.url.clone();
                    self.follow_link(&url)?;
                }
            }
            Overlay::Files => self.open_entry(key)?,
            Overlay::None | Overlay::Help => {}
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Picker
    // ------------------------------------------------------------------

    fn picker_items(&self) -> Vec<PickItem> {
        let all: Vec<PickItem> = match self.overlay_or_kind() {
            Overlay::Toc => self
                .rendered
                .headings
                .iter()
                .enumerate()
                .map(|(idx, h)| PickItem {
                    label: h.text.clone(),
                    detail: String::new(),
                    indent: h.level.saturating_sub(1).min(5),
                    key: idx,
                })
                .collect(),
            Overlay::Links => self
                .rendered
                .links
                .iter()
                .enumerate()
                .map(|(idx, l)| PickItem {
                    label: l.text.clone(),
                    detail: l.url.clone(),
                    indent: 0,
                    key: idx,
                })
                .collect(),
            Overlay::Files => self
                .entries
                .iter()
                .enumerate()
                .map(|(idx, e)| PickItem {
                    label: e.label(),
                    detail: String::new(),
                    indent: 0,
                    key: idx,
                })
                .collect(),
            Overlay::None | Overlay::Help => Vec::new(),
        };
        let active = if self.picker.filter_mode {
            self.picker.draft.trim()
        } else {
            self.picker.filter.trim()
        };
        if active.is_empty() {
            return all;
        }
        let matcher = SearchMatcher::new(active);
        all.into_iter()
            .filter(|item| matcher.contains(&item.label) || matcher.contains(&item.detail))
            .collect()
    }

    fn overlay_or_kind(&self) -> Overlay {
        if self.overlay == Overlay::None {
            self.picker_kind
        } else {
            self.overlay
        }
    }

    fn selected_item(&self) -> Option<PickItem> {
        self.picker_items().get(self.picker.idx).cloned()
    }

    fn picker_body_height(&self) -> usize {
        self.height.saturating_sub(7).clamp(3, 30)
    }

    // ------------------------------------------------------------------
    // Documents, links, history
    // ------------------------------------------------------------------

    fn current_line_for_history(&self) -> usize {
        self.display
            .at(min(self.offset, self.display.len().saturating_sub(1)))
    }

    fn switch_to(
        &mut self,
        doc: Doc,
        entry: Option<usize>,
        push_history: bool,
    ) -> Result<(), String> {
        if push_history {
            self.history.push(HistoryEntry {
                doc: self.doc.clone(),
                entry: self.entry,
                line: self.current_line_for_history(),
            });
            if self.history.len() > 100 {
                self.history.remove(0);
            }
        }
        self.doc = doc;
        self.entry = entry;
        self.offset = 0;
        self.search_idx = 0;
        self.rerender()?;
        self.offset = 0;
        if !self.search_query.is_empty() {
            self.recompute_search();
        }
        Ok(())
    }

    fn open_entry(&mut self, idx: usize) -> Result<(), String> {
        let Some(source) = self.entries.get(idx).cloned() else {
            return Ok(());
        };
        if self.entry == Some(idx) {
            return Ok(());
        }
        match load_source(&source) {
            Ok(doc) => {
                self.switch_to(doc, Some(idx), true)?;
                self.set_status(&format!(
                    "[{}/{}] {}",
                    idx + 1,
                    self.entries.len(),
                    source.label()
                ));
            }
            Err(err) => self.set_status(&err),
        }
        Ok(())
    }

    fn cycle_entry(&mut self, delta: isize) -> Result<(), String> {
        if self.entries.len() <= 1 {
            self.set_status("Only one document");
            return Ok(());
        }
        let len = self.entries.len() as isize;
        let current = self.entry.map(|e| e as isize).unwrap_or(-1);
        let next = (current + delta).rem_euclid(len) as usize;
        self.open_entry(next)
    }

    fn go_back(&mut self) -> Result<(), String> {
        let Some(prev) = self.history.pop() else {
            self.set_status("No previous document");
            return Ok(());
        };
        let doc = match prev.doc.path.as_deref() {
            Some(path) => load_path(path).unwrap_or(prev.doc),
            None => prev.doc,
        };
        self.switch_to(doc, prev.entry, false)?;
        self.set_offset_for_rendered_line(prev.line);
        self.set_status(&format!("Back: {}", self.doc.title));
        Ok(())
    }

    fn base_dir(&self) -> PathBuf {
        self.doc
            .path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn follow_link(&mut self, url: &str) -> Result<(), String> {
        let url = url.trim();
        if url.is_empty() {
            return Ok(());
        }
        if let Some(anchor) = url.strip_prefix('#') {
            if !self.jump_to_heading_query(&percent_decode(anchor)) {
                self.set_status(&format!("Anchor not found: #{anchor}"));
            }
            return Ok(());
        }
        if is_web_url(url) {
            match open_external(url) {
                Ok(()) => self.set_status(&format!("Opened {url}")),
                Err(err) => self.set_status(&format!("{err} (press l then y to copy the URL)")),
            }
            return Ok(());
        }
        if has_scheme(url) {
            self.set_status(&format!("Unsupported link: {url}"));
            return Ok(());
        }

        let (path_part, fragment) = match url.split_once('#') {
            Some((p, f)) => (p, Some(f)),
            None => (url, None),
        };
        let path_part = path_part.split('?').next().unwrap_or(path_part);
        let rel = percent_decode(path_part);
        let mut target = if let Some(stripped) = rel.strip_prefix('/') {
            find_repo_root(&self.base_dir()).join(stripped)
        } else {
            self.base_dir().join(&rel)
        };
        target = normalize_path(&target);
        if target.is_dir() {
            match ["README.md", "readme.md", "Readme.md", "index.md"]
                .iter()
                .map(|name| target.join(name))
                .find(|p| p.is_file())
            {
                Some(found) => target = found,
                None => {
                    self.set_status(&format!("No README.md in {}", display_path(&target)));
                    return Ok(());
                }
            }
        }
        if !target.exists() {
            self.set_status(&format!("Not found: {}", display_path(&target)));
            return Ok(());
        }
        if is_markdown_path(&target) {
            match load_path(&target) {
                Ok(doc) => {
                    let entry = self.entry_index_for(&target);
                    self.switch_to(doc, entry, true)?;
                    let mut message =
                        format!("Opened {} (Backspace to go back)", display_path(&target));
                    if let Some(fragment) = fragment.filter(|f| !f.is_empty()) {
                        if !self.jump_to_heading_query(&percent_decode(fragment)) {
                            message = format!(
                                "Opened {}; anchor not found: #{fragment}",
                                display_path(&target)
                            );
                        }
                    }
                    self.set_status(&message);
                }
                Err(err) => self.set_status(&err),
            }
            return Ok(());
        }
        if is_external_viewable(&target) {
            let shown = display_path(&target);
            // Absolute path: a relative name like `-x.png` must not look like an option.
            let absolute = fs::canonicalize(&target).unwrap_or_else(|_| target.clone());
            match open_external(&absolute.to_string_lossy()) {
                Ok(()) => self.set_status(&format!("Opened {shown}")),
                Err(err) => self.set_status(&err),
            }
            return Ok(());
        }
        self.set_status(&format!("Not a Markdown file: {}", display_path(&target)));
        Ok(())
    }

    fn entry_index_for(&self, path: &Path) -> Option<usize> {
        let canon = fs::canonicalize(path).ok()?;
        self.entries.iter().position(|e| match e {
            DocSource::Path(p) => fs::canonicalize(p).ok().as_ref() == Some(&canon),
            DocSource::Text { .. } => false,
        })
    }

    fn jump_to_heading_query(&mut self, query: &str) -> bool {
        match self.rendered.find_heading(query) {
            Some(idx) => {
                let line = self.rendered.headings[idx].line;
                self.set_offset_for_rendered_line(line);
                true
            }
            None => false,
        }
    }

    fn reload(&mut self, manual: bool) -> Result<(), String> {
        let Some(path) = self.doc.path.clone() else {
            if manual {
                self.set_status("Cannot reload: document was read from stdin");
            }
            return Ok(());
        };
        match load_path(&path) {
            Ok(doc) => {
                let anchor = self.anchor_line();
                let changed = doc.markdown != self.doc.markdown;
                self.doc = doc;
                if changed {
                    self.rerender()?;
                    self.restore_anchor(anchor);
                }
                self.set_status(if manual {
                    "Reloaded"
                } else {
                    "Reloaded (file changed on disk)"
                });
            }
            Err(err) => self.set_status(&err),
        }
        Ok(())
    }

    fn reload_if_changed(&mut self) -> Result<(), String> {
        let Some(path) = self.doc.path.as_deref() else {
            return Ok(());
        };
        let mtime = fs::metadata(path).and_then(|m| m.modified()).ok();
        if mtime.is_some() && mtime != self.doc.mtime {
            self.reload(false)?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Clipboard
    // ------------------------------------------------------------------

    fn copy_visible_code_block(&mut self) {
        if self.rendered.code_blocks.is_empty() {
            self.set_status("No code blocks in this document");
            return;
        }
        let (top, bottom) = self.visible_rendered_bounds();
        let visible: Vec<usize> = self
            .rendered
            .code_blocks
            .iter()
            .enumerate()
            .filter(|(_, cb)| cb.end_line >= top && cb.start_line <= bottom)
            .map(|(idx, _)| idx)
            .collect();
        if visible.is_empty() {
            self.set_status("No code block on screen");
            return;
        }
        let pick = match self.last_copied_block {
            Some(last) => match visible.iter().position(|idx| *idx == last) {
                Some(pos) => visible[(pos + 1) % visible.len()],
                None => visible[0],
            },
            None => visible[0],
        };
        self.last_copied_block = Some(pick);
        let block = self.rendered.code_blocks[pick].clone();
        let lines = block.code.lines().count();
        let lang = block.language.as_deref().unwrap_or("code");
        let more = if visible.len() > 1 {
            " - y again for next"
        } else {
            ""
        };
        self.copy_to_clipboard(
            &block.code,
            &format!(
                "Copied {lang} block ({lines} line{}){more}",
                if lines == 1 { "" } else { "s" }
            ),
        );
    }

    fn copy_to_clipboard(&mut self, text: &str, message: &str) {
        let seq = osc52_sequence(text, env::var_os("TMUX").is_some());
        let mut stdout = io::stdout();
        match stdout
            .write_all(seq.as_bytes())
            .and_then(|_| stdout.flush())
        {
            Ok(()) => self.set_status(message),
            Err(err) => self.set_status(&format!("copy failed: {err}")),
        }
    }

    // ------------------------------------------------------------------
    // Drawing
    // ------------------------------------------------------------------

    fn draw(&self, stdout: &mut io::Stdout) -> Result<(), String> {
        let res: io::Result<()> = (|| {
            // Compose the whole frame first and write it at once: one write
            // instead of many small ones through stdout's line buffer.
            let mut frame: Vec<u8> = Vec::with_capacity(self.width * self.height * 4);
            match self.overlay {
                Overlay::Help => {
                    queue!(frame, MoveTo(0, 0), Clear(ClearType::All))?;
                    self.draw_help(&mut frame)?;
                }
                Overlay::Toc | Overlay::Links | Overlay::Files => {
                    queue!(frame, MoveTo(0, 0), Clear(ClearType::All))?;
                    self.draw_picker(&mut frame)?;
                }
                Overlay::None => self.draw_main(&mut frame)?,
            }
            stdout.write_all(&frame)?;
            stdout.flush()
        })();
        res.map_err(|err| format!("draw: {err}"))
    }

    fn draw_main(&self, stdout: &mut Vec<u8>) -> io::Result<()> {
        self.draw_bar(stdout, 0, &self.header_text(), true)?;
        let content_height = self.page_size();
        let start = min(self.offset, self.display.len());
        let end = min(start + content_height, self.display.len());
        let text_width = self.body_text_width();
        let matcher =
            (!self.search_query.is_empty()).then(|| SearchMatcher::new(&self.search_query));

        for row in 0..content_height {
            queue!(stdout, MoveTo(0, (row + 1) as u16))?;
            if start + row < end {
                let idx = self.display.at(start + row);
                let marker = self.marker(idx);
                queue!(
                    stdout,
                    SetForegroundColor(Color::DarkGrey),
                    Print(format!(" {marker} ")),
                    ResetColor
                )?;
                let mut line = self.rendered.lines.get(idx).cloned().unwrap_or_default();
                if let Some(matcher) = matcher.as_ref() {
                    if self.search_matches.binary_search(&idx).is_ok() {
                        let plain = self.plain.get(idx).map(String::as_str).unwrap_or("");
                        line = highlight_matches(
                            &line,
                            plain,
                            matcher,
                            self.search_current_line == Some(idx),
                        );
                    }
                }
                let line = fit_visible(&line, text_width);
                if self.is_heading_rendered_line(idx) {
                    queue!(
                        stdout,
                        SetBackgroundColor(Color::DarkGrey),
                        Print(line),
                        ResetColor
                    )?;
                } else {
                    queue!(stdout, Print(line), ResetColor)?;
                }
                queue!(
                    stdout,
                    SetForegroundColor(Color::DarkGrey),
                    Print(self.scrollbar_char(row, content_height)),
                    ResetColor
                )?;
            } else {
                queue!(stdout, Clear(ClearType::UntilNewLine))?;
            }
        }
        self.draw_bar(
            stdout,
            self.height.saturating_sub(1) as u16,
            &self.footer_text(),
            false,
        )
    }

    fn draw_help(&self, stdout: &mut Vec<u8>) -> io::Result<()> {
        let lines = [
            "Keys",
            "",
            "  q / Esc          quit",
            "  j/k, Up/Down     scroll",
            "  Space/f, b       page down / up (also PgDn/PgUp)",
            "  d/u              half page",
            "  g/G              top / bottom",
            "  [ / ]            previous / next heading",
            "  1-6 / 0          fold outline by heading level / show all",
            "  /                search (n/N next/prev, c clear)",
            "  t                table of contents",
            "  l                links: Enter follow/open, y copy URL",
            "  Backspace        back to previous document",
            "  Tab / Shift-Tab  next / previous file",
            "  F                file list",
            "  y                copy code block on screen (repeat: next)",
            "  r                reload (auto-reloads when the file changes)",
            "  / (in lists)     filter",
            "  ?                toggle this help",
        ];
        let width = lines.iter().map(|l| visible_width(l)).max().unwrap_or(0);
        let top = self.height.saturating_sub(lines.len()) / 2;
        let left = self.width.saturating_sub(width) / 2;
        for (idx, line) in lines.iter().enumerate() {
            if top + idx >= self.height {
                break;
            }
            queue!(
                stdout,
                MoveTo(left as u16, (top + idx) as u16),
                Print(truncate_visible(line, self.width.saturating_sub(left)))
            )?;
        }
        Ok(())
    }

    fn draw_picker(&self, stdout: &mut Vec<u8>) -> io::Result<()> {
        let kind = self.overlay;
        let all_count = match kind {
            Overlay::Toc => self.rendered.headings.len(),
            Overlay::Links => self.rendered.links.len(),
            Overlay::Files => self.entries.len(),
            _ => 0,
        };
        let title = match kind {
            Overlay::Toc => format!("Table of contents ({all_count})"),
            Overlay::Links => format!("Links ({all_count})"),
            Overlay::Files => format!("Files ({all_count})"),
            _ => String::new(),
        };
        let items = self.picker_items();
        let filter = if self.picker.filter_mode {
            format!("/{}", self.picker.draft)
        } else if self.picker.filter.is_empty() {
            "/ filter".to_string()
        } else {
            format!("/{} ({}/{})", self.picker.filter, items.len(), all_count)
        };
        let hint = if self.picker.filter_mode {
            "type to filter  Enter apply  Esc cancel"
        } else {
            match kind {
                Overlay::Links => "j/k move  Enter follow/open  y copy URL  / filter  Esc close",
                Overlay::Files => "j/k move  Enter open  / filter  Esc close",
                _ => "j/k move  Enter jump  / filter  Esc close",
            }
        };

        let body = self.picker_body_height();
        let box_width = min(self.width.saturating_sub(4).max(10), 100);
        let left = self.width.saturating_sub(box_width) / 2;
        let total_height = body + 5;
        let top = self.height.saturating_sub(total_height) / 2;

        let mut row = top;
        let mut put =
            |stdout: &mut Vec<u8>, text: &str, selected: bool, dim: bool| -> io::Result<()> {
                if row >= self.height {
                    return Ok(());
                }
                queue!(stdout, MoveTo(left as u16, row as u16))?;
                if selected {
                    queue!(stdout, SetAttribute(Attribute::Reverse))?;
                }
                if dim {
                    queue!(stdout, SetForegroundColor(Color::DarkGrey))?;
                }
                let shown = if selected {
                    fit_visible(text, box_width)
                } else {
                    truncate_visible(text, box_width)
                };
                queue!(
                    stdout,
                    Print(shown),
                    SetAttribute(Attribute::Reset),
                    ResetColor
                )?;
                row += 1;
                Ok(())
            };

        put(stdout, &title, false, false)?;
        put(stdout, &filter, false, true)?;
        put(stdout, "", false, false)?;
        if items.is_empty() {
            put(
                stdout,
                if all_count == 0 {
                    "  (nothing here)"
                } else {
                    "  (no matches)"
                },
                false,
                true,
            )?;
        } else {
            let start = self.picker.idx.saturating_sub(body / 2);
            let start = min(start, items.len().saturating_sub(body));
            let end = min(items.len(), start + body);
            for (offset, item) in items[start..end].iter().enumerate() {
                let actual = start + offset;
                let selected = actual == self.picker.idx;
                let current = match kind {
                    Overlay::Files => self.entry == Some(item.key),
                    Overlay::Toc => self.current_heading_index() == Some(item.key),
                    _ => false,
                };
                let prefix = if selected {
                    "> "
                } else if current {
                    "* "
                } else {
                    "  "
                };
                let mut text = format!("{prefix}{}{}", "  ".repeat(item.indent), item.label);
                if !item.detail.is_empty() && item.detail != item.label {
                    text.push_str("  ");
                    text.push_str(&format!("<{}>", item.detail));
                }
                put(stdout, &text, selected, false)?;
            }
        }
        put(stdout, "", false, false)?;
        put(stdout, hint, false, true)?;
        Ok(())
    }

    fn draw_bar(&self, stdout: &mut Vec<u8>, row: u16, text: &str, header: bool) -> io::Result<()> {
        let bg = if header {
            Color::DarkBlue
        } else {
            Color::DarkGrey
        };
        queue!(
            stdout,
            MoveTo(0, row),
            SetBackgroundColor(bg),
            SetForegroundColor(Color::White),
            Print(fit_visible(text, self.width)),
            ResetColor
        )
    }

    fn header_text(&self) -> String {
        let pct = self.progress_percent();
        let right = if self.fold_level > 0 {
            format!("{pct:>3}%  H{}", self.fold_level)
        } else {
            format!("{pct:>3}%")
        };
        let mut left = self.doc.title.trim().to_string();
        if left.is_empty() {
            left = "md".to_string();
        }
        if self.entries.len() > 1 {
            if let Some(entry) = self.entry {
                left = format!("[{}/{}] {left}", entry + 1, self.entries.len());
            }
        }
        if let Some(breadcrumb) = self.current_breadcrumb() {
            left.push_str(" > ");
            left.push_str(&breadcrumb);
        }
        join_left_right(&left, &right, self.width)
    }

    fn footer_text(&self) -> String {
        let meta = if self.fold_level > 0 {
            let (s, e, t) = self.visible_outline_range();
            let (ds, de, dt) = self.visible_doc_range();
            format!("doc {ds}-{de}/{dt} | ol {s}-{e}/{t}")
        } else {
            let (s, e, t) = self.visible_doc_range();
            format!("doc {s}-{e}/{t}")
        };

        let left = if self.search_mode {
            if self.search_draft.trim().is_empty() {
                format!("/{}", self.search_draft)
            } else {
                format!(
                    "/{} ({}) Enter jump Esc cancel",
                    self.search_draft,
                    self.search_matches.len()
                )
            }
        } else if !self.status_message.is_empty() {
            self.status_message.clone()
        } else if !self.search_query.is_empty() {
            format!(
                "/{} {}/{} (n/N)",
                self.search_query,
                self.current_match_number(),
                self.search_matches.len()
            )
        } else {
            let mut hint = "q quit  ? help  / search  t toc  l links  y copy".to_string();
            if !self.history.is_empty() {
                hint.push_str("  BS back");
            }
            if self.entries.len() > 1 {
                hint.push_str("  F files");
            }
            hint
        };
        join_left_right(&left, &meta, self.width)
    }

    // ------------------------------------------------------------------
    // Search
    // ------------------------------------------------------------------

    fn set_search_query(&mut self, query: &str) {
        self.set_search_query_no_jump(query);
        if let Some(line) = self.search_current_line {
            self.set_offset_for_rendered_line(line);
        }
    }

    fn set_search_query_no_jump(&mut self, query: &str) {
        self.search_query = query.trim().to_string();
        self.search_draft = self.search_query.clone();
        self.search_idx = 0;
        self.search_current_line = None;
        self.search_matches.clear();
        if self.search_query.is_empty() {
            return;
        }
        self.recompute_search();
        if self.search_matches.is_empty() {
            return;
        }
        let top = self.anchor_line().saturating_sub(1);
        let idx = self
            .search_matches
            .iter()
            .position(|line| *line >= top)
            .unwrap_or(0);
        self.search_idx = idx;
        self.search_current_line = self.search_matches.get(idx).copied();
    }

    fn recompute_search(&mut self) {
        self.search_matches.clear();
        self.search_current_line = None;
        if self.search_query.is_empty() {
            return;
        }
        let matcher = SearchMatcher::new(&self.search_query);
        for (idx, line) in self.plain.iter().enumerate() {
            if matcher.contains(line) {
                self.search_matches.push(idx);
            }
        }
        if !self.search_matches.is_empty() {
            self.search_idx = min(self.search_idx, self.search_matches.len() - 1);
            self.search_current_line = Some(self.search_matches[self.search_idx]);
        }
    }

    fn jump_next_match(&mut self, delta: isize) {
        if self.search_matches.is_empty() {
            if !self.search_query.is_empty() {
                self.set_status(&format!("Not found: {}", self.search_query));
            }
            return;
        }
        let len = self.search_matches.len() as isize;
        let next = self.search_idx as isize + delta;
        if next >= len {
            self.set_status("Search wrapped to top");
        } else if next < 0 {
            self.set_status("Search wrapped to bottom");
        }
        self.search_idx = next.rem_euclid(len) as usize;
        let line = self.search_matches[self.search_idx];
        self.search_current_line = Some(line);
        self.set_offset_for_rendered_line(line);
    }

    fn current_match_number(&self) -> usize {
        if self.search_matches.is_empty() {
            0
        } else {
            self.search_idx + 1
        }
    }

    fn marker(&self, rendered_idx: usize) -> char {
        if self.search_current_line == Some(rendered_idx) {
            '>'
        } else if !self.search_query.is_empty()
            && self.search_matches.binary_search(&rendered_idx).is_ok()
        {
            '*'
        } else if self.is_heading_rendered_line(rendered_idx) {
            '#'
        } else {
            ' '
        }
    }

    // ------------------------------------------------------------------
    // Navigation / outline
    // ------------------------------------------------------------------

    fn jump_heading(&mut self, delta: isize) {
        let mut lines: Vec<usize> = self.rendered.headings.iter().map(|h| h.line).collect();
        if self.fold_level > 0 {
            let filtered: Vec<usize> = self
                .rendered
                .headings
                .iter()
                .filter(|h| h.level <= self.fold_level)
                .map(|h| h.line)
                .collect();
            if !filtered.is_empty() {
                lines = filtered;
            }
        }
        if lines.is_empty() {
            return;
        }
        let anchor = self.anchor_line();
        let current = lines.iter().rposition(|line| *line <= anchor);
        let next = match (current, delta > 0) {
            (None, true) => 0,
            (None, false) => 0,
            (Some(idx), true) => min(idx + 1, lines.len() - 1),
            (Some(idx), false) => idx.saturating_sub(1),
        };
        self.set_offset_for_rendered_line(lines[next]);
    }

    fn rebuild_display(&mut self) {
        let total = self.rendered.lines.len();
        if total == 0 {
            self.display = DisplayIndex::Identity(0);
            self.offset = 0;
            return;
        }
        let anchor = self.anchor_line();
        let idx: Vec<usize> = if self.fold_level == 0 {
            Vec::new()
        } else {
            self.rendered
                .headings
                .iter()
                .filter(|h| h.level <= self.fold_level)
                .map(|h| h.line)
                .collect()
        };
        if idx.is_empty() {
            let was_folded = matches!(self.display, DisplayIndex::List(_));
            self.display = DisplayIndex::Identity(total);
            if was_folded {
                self.offset = anchor;
            }
            self.clamp_offset();
            return;
        }
        let mut target = idx[0];
        for value in &idx {
            if *value <= anchor {
                target = *value;
            } else {
                break;
            }
        }
        self.display = DisplayIndex::List(idx);
        self.offset = self.display_row_for_rendered_line(target);
        self.clamp_offset();
    }

    fn display_row_for_rendered_line(&self, line: usize) -> usize {
        match &self.display {
            DisplayIndex::Identity(size) => min(line, size.saturating_sub(1)),
            DisplayIndex::List(lines) => lines.iter().rposition(|v| *v <= line).unwrap_or(0),
        }
    }

    fn set_offset_for_rendered_line(&mut self, line: usize) {
        self.offset = self.display_row_for_rendered_line(line);
        self.clamp_offset();
    }

    fn restore_anchor(&mut self, anchor: usize) {
        // `anchor_line` points one row below the top; undo that.
        self.set_offset_for_rendered_line(anchor.saturating_sub(1));
    }

    fn anchor_line(&self) -> usize {
        if self.display.len() == 0 {
            return 0;
        }
        self.display
            .at(min(self.offset + 1, self.display.len() - 1))
    }

    fn visible_rendered_bounds(&self) -> (usize, usize) {
        if self.display.len() == 0 {
            return (0, 0);
        }
        let top = self.display.at(self.offset);
        let bottom_row = min(self.offset + self.page_size(), self.display.len()) - 1;
        (top, self.display.at(bottom_row))
    }

    fn is_rendered_line_visible(&self, line: usize) -> bool {
        let (top, bottom) = self.visible_rendered_bounds();
        line >= top && line <= bottom
    }

    fn current_heading_index(&self) -> Option<usize> {
        let anchor = self.anchor_line();
        self.rendered
            .headings
            .iter()
            .rposition(|h| h.line <= anchor)
    }

    fn current_breadcrumb(&self) -> Option<String> {
        let idx = self.current_heading_index()?;
        let headings = &self.rendered.headings;
        let mut chain = vec![headings[idx].text.trim().to_string()];
        let mut level = headings[idx].level;
        for h in headings[..idx].iter().rev() {
            if level <= 1 {
                break;
            }
            if h.level < level {
                chain.push(h.text.trim().to_string());
                level = h.level;
            }
        }
        chain.reverse();
        Some(chain.join(" > "))
    }

    fn is_heading_rendered_line(&self, idx: usize) -> bool {
        self.rendered
            .headings
            .binary_search_by_key(&idx, |h| h.line)
            .is_ok()
    }

    fn body_text_width(&self) -> usize {
        max(10, self.width.saturating_sub(4))
    }

    fn page_size(&self) -> usize {
        max(1, self.height.saturating_sub(2))
    }

    fn max_offset(&self) -> usize {
        self.display.len().saturating_sub(self.page_size())
    }

    fn clamp_offset(&mut self) {
        self.offset = min(self.offset, self.max_offset());
    }

    fn scroll_by(&mut self, delta: isize) {
        if delta < 0 {
            self.offset = self.offset.saturating_sub(delta.unsigned_abs());
        } else {
            self.offset = self.offset.saturating_add(delta as usize);
        }
        self.clamp_offset();
    }

    fn progress_percent(&self) -> usize {
        let total = self.rendered.lines.len();
        if total == 0 {
            return 0;
        }
        let doc_max_off = total.saturating_sub(self.page_size());
        if doc_max_off == 0 {
            return 100;
        }
        let (top, _, _) = self.visible_doc_range();
        min(100, top.saturating_sub(1) * 100 / doc_max_off)
    }

    fn visible_doc_range(&self) -> (usize, usize, usize) {
        let total = self.rendered.lines.len();
        if total == 0 || self.display.len() == 0 {
            return (0, 0, total);
        }
        let top = min(self.offset, self.display.len() - 1);
        let bottom = min(self.offset + self.page_size() - 1, self.display.len() - 1);
        let start = min(self.display.at(top) + 1, total);
        let end = min(max(self.display.at(bottom) + 1, start), total);
        (start, end, total)
    }

    fn visible_outline_range(&self) -> (usize, usize, usize) {
        let total = self.display.len();
        if total == 0 {
            return (0, 0, 0);
        }
        let start = min(self.offset + 1, total);
        let end = min(max(self.offset + self.page_size(), start), total);
        (start, end, total)
    }

    fn scrollbar_char(&self, row: usize, visible: usize) -> &'static str {
        let total = self.display.len();
        if total <= visible || visible == 0 {
            return " ";
        }
        let thumb_size = max(1, min(visible, (visible * visible) / total));
        let max_off = self.max_offset();
        let top = if max_off > 0 && visible > thumb_size {
            self.offset * (visible - thumb_size) / max_off
        } else {
            0
        };
        if row >= top && row < top + thumb_size {
            "|"
        } else {
            "."
        }
    }

    fn set_status(&mut self, message: &str) {
        self.status_message = message.to_string();
        self.status_until = Some(Instant::now() + Duration::from_millis(2500));
        self.dirty = true;
    }
}

// ----------------------------------------------------------------------
// Loading
// ----------------------------------------------------------------------

fn load_source(source: &DocSource) -> Result<Doc, String> {
    match source {
        DocSource::Path(path) => load_path(path),
        DocSource::Text { title, markdown } => Ok(Doc {
            title: title.clone(),
            path: None,
            markdown: markdown.clone(),
            mtime: None,
        }),
    }
}

fn load_path(path: &Path) -> Result<Doc, String> {
    let bytes = fs::read(path)
        .map_err(|err| format!("read file {:?}: {err}", path.display().to_string()))?;
    let mtime = fs::metadata(path).and_then(|m| m.modified()).ok();
    Ok(Doc {
        title: sanitize_control(
            &path
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| display_path(path)),
        )
        .into_owned(),
        path: Some(path.to_path_buf()),
        markdown: String::from_utf8_lossy(&bytes).into_owned(),
        mtime,
    })
}

fn display_path(path: &Path) -> String {
    let shown = env::current_dir()
        .ok()
        .and_then(|cwd| path.strip_prefix(&cwd).ok().map(Path::to_path_buf))
        .unwrap_or_else(|| path.to_path_buf());
    let s = sanitize_control(&shown.to_string_lossy()).into_owned();
    s.strip_prefix("./").map(str::to_string).unwrap_or(s)
}

/// Lexically normalize `a/./b/../c` -> `a/c` (without touching the disk).
fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

/// Nearest ancestor containing `.git` (for repo-absolute links like `/docs/x.md`).
fn find_repo_root(start: &Path) -> PathBuf {
    let abs = fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf());
    let mut cur = abs.as_path();
    loop {
        if cur.join(".git").exists() {
            return cur.to_path_buf();
        }
        match cur.parent() {
            Some(parent) => cur = parent,
            None => return env::current_dir().unwrap_or(abs),
        }
    }
}

// ----------------------------------------------------------------------
// Links & external programs
// ----------------------------------------------------------------------

fn is_web_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
}

fn has_scheme(url: &str) -> bool {
    match url.find(':') {
        Some(idx) if idx > 1 => url[..idx]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')),
        _ => false,
    }
}

/// Local files that are safe to hand to the system viewer (no executables/scripts).
fn is_external_viewable(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "bmp" | "ico" | "pdf" | "txt"
    )
}

fn open_external(target: &str) -> Result<(), String> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(target);
        c
    } else if cfg!(windows) {
        // rundll32 avoids `cmd /c start`, which would interpret `&` etc.
        let mut c = Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler").arg(target);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(target);
        c
    };
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("cannot launch opener: {err}"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(value) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// OSC 52 "set clipboard" sequence (wrapped for tmux passthrough if needed).
pub fn osc52_sequence(text: &str, tmux: bool) -> String {
    let seq = format!("\x1b]52;c;{}\x07", base64_encode(text.as_bytes()));
    if tmux {
        format!("\x1bPtmux;{}\x1b\\", seq.replace('\x1b', "\x1b\x1b"))
    } else {
        seq
    }
}

// ----------------------------------------------------------------------
// Text helpers
// ----------------------------------------------------------------------

struct SearchMatcher {
    needle: Vec<char>,
    /// `needle` as a string (for substring search without allocation).
    needle_str: String,
    sensitive: bool,
}

impl SearchMatcher {
    fn new(query: &str) -> Self {
        // Smart case: any uppercase letter makes the search case-sensitive.
        let sensitive = query.chars().any(char::is_uppercase);
        let needle: Vec<char> = query
            .chars()
            .map(|c| if sensitive { c } else { fold_char(c) })
            .collect();
        let needle_str = needle.iter().collect();
        Self {
            needle,
            needle_str,
            sensitive,
        }
    }

    fn fold(&self, c: char) -> char {
        if self.sensitive {
            c
        } else {
            fold_char(c)
        }
    }

    /// Same result as `!self.find_ranges(line).is_empty()`, without
    /// collecting the line or the ranges (a char-sequence match is a UTF-8
    /// substring match).
    fn contains(&self, line: &str) -> bool {
        if self.needle.is_empty() {
            return false;
        }
        if self.sensitive {
            return line.contains(self.needle_str.as_str());
        }
        if line.is_ascii() {
            // `fold_char` on ASCII is `to_ascii_lowercase`; a folded ASCII
            // line cannot contain a non-ASCII needle.
            return self.needle_str.is_ascii()
                && line.to_ascii_lowercase().contains(self.needle_str.as_str());
        }
        let folded: String = line.chars().map(fold_char).collect();
        folded.contains(self.needle_str.as_str())
    }

    /// Non-overlapping match ranges as char indices `[start, end)`.
    fn find_ranges(&self, line: &str) -> Vec<(usize, usize)> {
        let hay: Vec<char> = line.chars().map(|c| self.fold(c)).collect();
        let n = self.needle.len();
        let mut out = Vec::new();
        if n == 0 || hay.len() < n {
            return out;
        }
        let mut i = 0;
        while i + n <= hay.len() {
            if hay[i..i + n] == self.needle[..] {
                out.push((i, i + n));
                i += n;
            } else {
                i += 1;
            }
        }
        out
    }
}

fn fold_char(c: char) -> char {
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

/// Highlight search matches inside an ANSI-styled line.
fn highlight_matches(ansi: &str, plain: &str, matcher: &SearchMatcher, current: bool) -> String {
    let ranges = matcher.find_ranges(plain);
    if ranges.is_empty() {
        return ansi.to_string();
    }
    let (on, off) = if current {
        ("\x1b[30;43m", "\x1b[0m")
    } else {
        ("\x1b[7m", "\x1b[27m")
    };
    let mut out = String::with_capacity(ansi.len() + ranges.len() * 12);
    let mut pos = 0usize; // plain char index
    let mut range_idx = 0usize;
    let mut in_match = false;
    let mut chars = ansi.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            out.push(ch);
            for c in chars.by_ref() {
                out.push(c);
                if ('\x40'..='\x7e').contains(&c) && c != '[' {
                    break;
                }
            }
            if in_match {
                out.push_str(on);
            }
            continue;
        }
        if in_match && range_idx < ranges.len() && pos == ranges[range_idx].1 {
            out.push_str(off);
            in_match = false;
            range_idx += 1;
        }
        if !in_match && range_idx < ranges.len() && pos == ranges[range_idx].0 {
            out.push_str(on);
            in_match = true;
        }
        out.push(ch);
        pos += 1;
    }
    if in_match {
        out.push_str(off);
    }
    out
}

fn join_left_right(left: &str, right: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let right_width = visible_width(right);
    let left_width = width.saturating_sub(right_width + 1);
    let left = truncate_visible(left, left_width);
    let gap = width.saturating_sub(visible_width(&left) + right_width);
    format!("{left}{}{right}", " ".repeat(gap))
}

/// Truncate to `width` display columns, keeping ANSI styling; adds "..."
/// when text is cut. Never pads.
fn truncate_visible(input: &str, width: usize) -> String {
    if visible_width(input) <= width {
        return input.to_string();
    }
    if width <= 3 {
        return ".".repeat(width);
    }
    let mut out = cut_ansi(input, width - 3);
    out.push_str("...");
    out
}

/// Truncate/pad to exactly `width` display columns, keeping ANSI styling.
fn fit_visible(input: &str, width: usize) -> String {
    let truncated = truncate_visible(input, width);
    let used = visible_width(&truncated);
    format!("{truncated}{}", " ".repeat(width.saturating_sub(used)))
}

fn cut_ansi(input: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    let mut had_escape = false;
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            had_escape = true;
            out.push(ch);
            for c in chars.by_ref() {
                out.push(c);
                if ('\x40'..='\x7e').contains(&c) && c != '[' {
                    break;
                }
            }
            continue;
        }
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > width {
            break;
        }
        used += w;
        out.push(ch);
    }
    if had_escape {
        out.push_str("\x1b[0m");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(md: &str, height: usize) -> Model {
        let doc = Doc {
            title: "t.md".to_string(),
            path: None,
            markdown: md.to_string(),
            mtime: None,
        };
        let mut m = Model::new(
            vec![DocSource::Text {
                title: "t.md".to_string(),
                markdown: md.to_string(),
            }],
            doc,
            RenderOptions {
                style: "dark".to_string(),
                width: 0,
            },
        );
        m.width = 60;
        m.height = height;
        m.rerender().unwrap();
        m
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn long_doc() -> String {
        let mut md = String::new();
        for section in ["Alpha", "Beta", "Gamma"] {
            md.push_str(&format!("## {section}\n\n"));
            for i in 0..30 {
                md.push_str(&format!("{section} line {i}\n\n"));
            }
        }
        md
    }

    #[test]
    fn heading_navigation_uses_rendered_positions() {
        let mut m = model(&long_doc(), 12);
        m.handle_key(key(KeyCode::Char(']'))).unwrap();
        let top = m.display.at(m.offset);
        assert_eq!(m.plain[top], "Beta");
        m.handle_key(key(KeyCode::Char(']'))).unwrap();
        assert_eq!(m.plain[m.display.at(m.offset)], "Gamma");
        m.handle_key(key(KeyCode::Char('['))).unwrap();
        assert_eq!(m.plain[m.display.at(m.offset)], "Beta");
        assert_eq!(m.current_breadcrumb().as_deref(), Some("Beta"));
    }

    #[test]
    fn toc_jumps_to_setext_and_inline_markup_headings() {
        let md = format!(
            "Intro\n=====\n\n{}\n## Use `md` here\n\nend\n",
            "x\n\n".repeat(40)
        );
        let mut m = model(&md, 10);
        m.handle_key(key(KeyCode::Char('t'))).unwrap();
        assert_eq!(m.overlay, Overlay::Toc);
        let items = m.picker_items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].label, "Use md here");
        m.handle_key(key(KeyCode::Char('j'))).unwrap();
        m.handle_key(key(KeyCode::Enter)).unwrap();
        assert_eq!(m.overlay, Overlay::None);
        let (top, bottom) = m.visible_rendered_bounds();
        let line = m.rendered.headings[1].line;
        assert!(line >= top && line <= bottom);
    }

    #[test]
    fn picker_filter_narrows_items() {
        let mut m = model(&long_doc(), 20);
        m.handle_key(key(KeyCode::Char('t'))).unwrap();
        m.handle_key(key(KeyCode::Char('/'))).unwrap();
        for ch in "gam".chars() {
            m.handle_key(key(KeyCode::Char(ch))).unwrap();
        }
        m.handle_key(key(KeyCode::Enter)).unwrap();
        let items = m.picker_items();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "Gamma");
    }

    #[test]
    fn search_finds_and_cycles_matches() {
        let mut m = model(&long_doc(), 12);
        m.handle_key(key(KeyCode::Char('/'))).unwrap();
        for ch in "line 29".chars() {
            m.handle_key(key(KeyCode::Char(ch))).unwrap();
        }
        m.handle_key(key(KeyCode::Enter)).unwrap();
        assert_eq!(m.search_matches.len(), 3);
        let first = m.search_current_line.unwrap();
        assert!(m.is_rendered_line_visible(first));
        m.handle_key(key(KeyCode::Char('n'))).unwrap();
        assert_ne!(m.search_current_line.unwrap(), first);
    }

    #[test]
    fn anchor_links_jump_within_document() {
        let md = format!(
            "[go](#target-section)\n\n{}## Target Section\n\nhere\n",
            "x\n\n".repeat(40)
        );
        let mut m = model(&md, 10);
        assert_eq!(m.rendered.links.len(), 1);
        m.follow_link("#target-section").unwrap();
        let line = m.rendered.headings[0].line;
        assert!(m.is_rendered_line_visible(line));
    }

    #[test]
    fn following_relative_markdown_link_and_back() {
        let dir = env::temp_dir().join(format!("md-test-{}", std::process::id()));
        let _ = fs::create_dir_all(dir.join("docs"));
        let a = dir.join("a.md");
        let b = dir.join("docs/b.md");
        fs::write(&a, "# A\n\n[to b](docs/b.md#second)\n").unwrap();
        let filler = "x\n\n".repeat(40);
        fs::write(&b, format!("# B\n\n{filler}## Second\n\nok\n")).unwrap();

        let doc = load_path(&a).unwrap();
        let mut m = Model::new(
            vec![DocSource::Path(a.clone())],
            doc,
            RenderOptions {
                style: "dark".to_string(),
                width: 0,
            },
        );
        m.width = 60;
        m.height = 10;
        m.rerender().unwrap();
        m.follow_link("docs/b.md#second").unwrap();
        assert_eq!(m.doc.title, "b.md");
        assert_eq!(m.entry, None);
        let second = m.rendered.headings[1].line;
        assert!(m.is_rendered_line_visible(second));
        m.handle_key(key(KeyCode::Backspace)).unwrap();
        assert_eq!(m.doc.title, "a.md");
        assert_eq!(m.entry, Some(0));

        m.follow_link("missing.md").unwrap();
        assert!(m.status_message.starts_with("Not found"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tab_cycles_through_entries_and_reload_detects_changes() {
        let dir = env::temp_dir().join(format!("md-test-tab-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let a = dir.join("a.md");
        let b = dir.join("b.md");
        fs::write(&a, "# A\n").unwrap();
        fs::write(&b, "# B\n").unwrap();
        let entries = vec![DocSource::Path(a.clone()), DocSource::Path(b.clone())];
        let doc = load_path(&a).unwrap();
        let mut m = Model::new(
            entries,
            doc,
            RenderOptions {
                style: "dark".to_string(),
                width: 0,
            },
        );
        m.width = 60;
        m.height = 10;
        m.rerender().unwrap();
        m.handle_key(key(KeyCode::Tab)).unwrap();
        assert_eq!(m.doc.title, "b.md");
        assert_eq!(m.entry, Some(1));
        m.handle_key(key(KeyCode::Tab)).unwrap();
        assert_eq!(m.doc.title, "a.md");

        fs::write(&a, "# A changed\n").unwrap();
        m.doc.mtime = None; // force detection regardless of mtime granularity
        m.reload_if_changed().unwrap();
        assert_eq!(m.rendered.headings[0].text, "A changed");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_cycles_through_visible_code_blocks() {
        let md = "```sh\none\n```\n\n```sh\ntwo\n```\n";
        let mut m = model(md, 20);
        m.copy_visible_code_block();
        assert_eq!(m.last_copied_block, Some(0));
        m.copy_visible_code_block();
        assert_eq!(m.last_copied_block, Some(1));
        m.copy_visible_code_block();
        assert_eq!(m.last_copied_block, Some(0));
    }

    #[test]
    fn fold_shows_only_headings() {
        let mut m = model(&long_doc(), 10);
        m.handle_key(key(KeyCode::Char('2'))).unwrap();
        assert_eq!(m.display.len(), 3);
        m.handle_key(key(KeyCode::Char('0'))).unwrap();
        assert_eq!(m.display.len(), m.rendered.lines.len());
    }

    #[test]
    fn highlight_inserts_markers_around_matches() {
        let matcher = SearchMatcher::new("bc");
        let out = highlight_matches("a\x1b[1mbcd\x1b[0m", "abcd", &matcher, false);
        assert_eq!(out, "a\x1b[1m\x1b[7mbc\x1b[27md\x1b[0m");
    }

    #[test]
    fn search_is_smart_case_and_unicode_aware() {
        assert!(SearchMatcher::new("ÄBC").contains("xÄBCx"));
        assert!(!SearchMatcher::new("Abc").contains("abc"));
        assert!(SearchMatcher::new("äbc").contains("ÄBC"));
        assert_eq!(
            SearchMatcher::new("日本").find_ranges("こんにちは日本"),
            vec![(5, 7)]
        );
    }

    #[test]
    fn search_contains_agrees_with_find_ranges() {
        const ALPHABET: &[&str] = &[
            "a", "b", "A", "B", "z", " ", "-", "1", "é", "É", "ß", "\u{212A}", "k", "K", "İ", "i",
            "ı", "日", "本", "ǅ", "ǆ", "e\u{301}", "Σ", "σ", "ς",
        ];
        fn next(seed: &mut u64, n: usize) -> usize {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            (*seed % n as u64) as usize
        }
        fn gen(seed: &mut u64, max_len: usize) -> String {
            let len = next(seed, max_len);
            (0..len)
                .map(|_| ALPHABET[next(seed, ALPHABET.len())])
                .collect()
        }
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..20_000 {
            let needle = gen(&mut seed, 4);
            let hay = gen(&mut seed, 12);
            let matcher = SearchMatcher::new(&needle);
            assert_eq!(
                matcher.contains(&hay),
                !matcher.find_ranges(&hay).is_empty(),
                "needle {needle:?} hay {hay:?}"
            );
        }
    }

    #[test]
    fn fit_visible_handles_wide_chars_and_ansi() {
        let s = "\x1b[1m日本語テキスト\x1b[0m";
        let out = fit_visible(s, 9);
        assert_eq!(visible_width(&out), 9);
        assert!(out.starts_with("\x1b[1m日本"));
        assert_eq!(fit_visible("ab", 4), "ab  ");
    }

    #[test]
    fn base64_and_osc52() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(osc52_sequence("foo", false), "\x1b]52;c;Zm9v\x07");
        assert!(osc52_sequence("foo", true).starts_with("\x1bPtmux;\x1b\x1b]52"));
    }

    #[test]
    fn percent_decoding_and_schemes() {
        assert_eq!(percent_decode("my%20doc.md"), "my doc.md");
        assert_eq!(percent_decode("100%"), "100%");
        assert!(is_web_url("HTTPS://x"));
        assert!(has_scheme("ftp://x"));
        assert!(!has_scheme("docs/a.md"));
        assert!(!has_scheme("C:\\docs\\a.md"));
        assert!(!is_external_viewable(Path::new("evil.sh")));
        assert!(is_external_viewable(Path::new("logo.PNG")));
    }

    #[test]
    fn normalize_path_resolves_dots() {
        assert_eq!(
            normalize_path(Path::new("a/./b/../c.md")),
            PathBuf::from("a/c.md")
        );
        assert_eq!(normalize_path(Path::new("./x.md")), PathBuf::from("x.md"));
    }
}
