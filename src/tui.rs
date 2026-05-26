use std::cmp::{max, min};
use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers, MouseEventKind};
use crossterm::execute;
use crossterm::style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};

use crate::render::{is_fence_line, parse_heading, render_markdown, strip_ansi, RenderOptions};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Heading {
    pub level: usize,
    pub text: String,
    pub line: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadingLoc {
    pub heading: Heading,
    pub rendered_line: usize,
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

#[derive(Debug)]
struct Model {
    title: String,
    markdown: String,
    render_opts: RenderOptions,
    lines: Vec<String>,
    plain: Vec<String>,
    offset: usize,
    fold_level: usize,
    display: DisplayIndex,
    width: usize,
    height: usize,
    show_help: bool,
    show_toc: bool,
    headings: Vec<Heading>,
    heading_locs: Vec<HeadingLoc>,
    heading_line_set: Vec<bool>,
    toc_idx: usize,
    toc_filter_mode: bool,
    toc_filter_draft: String,
    toc_filter: String,
    search_mode: bool,
    search_saved_query: String,
    search_draft: String,
    search_query: String,
    search_matches: Vec<usize>,
    search_idx: usize,
    search_current_line: Option<usize>,
    status_message: String,
    status_until: Option<Instant>,
}

pub fn view_markdown(title: &str, markdown: &str, opts: RenderOptions) -> Result<(), String> {
    let mut stdout = io::stdout();
    terminal::enable_raw_mode().map_err(|err| format!("enable raw mode: {err}"))?;
    execute!(stdout, EnterAlternateScreen, Hide)
        .map_err(|err| format!("enter alternate screen: {err}"))?;

    let mut model = Model::new(title, markdown, opts);
    model.resize()?;
    model.render_current_width()?;

    let result = run_loop(&mut stdout, &mut model);

    let _ = execute!(stdout, Show, LeaveAlternateScreen, ResetColor);
    let _ = terminal::disable_raw_mode();
    result
}

fn run_loop(stdout: &mut io::Stdout, model: &mut Model) -> Result<(), String> {
    loop {
        model.draw(stdout)?;
        if event::poll(Duration::from_millis(150)).map_err(|err| format!("poll input: {err}"))? {
            match event::read().map_err(|err| format!("read input: {err}"))? {
                Event::Key(key) => {
                    if model.handle_key(key)? {
                        break;
                    }
                }
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => model.scroll_by(-3),
                    MouseEventKind::ScrollDown => model.scroll_by(3),
                    _ => {}
                },
                Event::Resize(_, _) => {
                    model.resize()?;
                    model.render_current_width()?;
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
        }
    }
    Ok(())
}

impl Model {
    fn new(title: &str, markdown: &str, render_opts: RenderOptions) -> Self {
        let headings = parse_headings(markdown);
        Self {
            title: title.to_string(),
            markdown: markdown.to_string(),
            render_opts,
            lines: Vec::new(),
            plain: Vec::new(),
            offset: 0,
            fold_level: 0,
            display: DisplayIndex::Identity(0),
            width: 80,
            height: 24,
            show_help: false,
            show_toc: false,
            headings,
            heading_locs: Vec::new(),
            heading_line_set: Vec::new(),
            toc_idx: 0,
            toc_filter_mode: false,
            toc_filter_draft: String::new(),
            toc_filter: String::new(),
            search_mode: false,
            search_saved_query: String::new(),
            search_draft: String::new(),
            search_query: String::new(),
            search_matches: Vec::new(),
            search_idx: 0,
            search_current_line: None,
            status_message: String::new(),
            status_until: None,
        }
    }

    fn resize(&mut self) -> Result<(), String> {
        let (w, h) = terminal::size().map_err(|err| format!("terminal size: {err}"))?;
        self.width = usize::from(w);
        self.height = usize::from(h);
        Ok(())
    }

    fn render_current_width(&mut self) -> Result<(), String> {
        let width = if self.render_opts.width == 0 {
            self.body_text_width()
        } else {
            self.render_opts.width
        };
        let rendered = render_markdown(
            &self.markdown,
            RenderOptions {
                style: self.render_opts.style.clone(),
                width,
            },
        )?;
        self.lines = split_lines(&rendered);
        self.plain = self.lines.iter().map(|line| strip_ansi(line)).collect();
        self.heading_locs = compute_heading_locs_from_rendered(&self.plain, &self.headings);
        self.heading_line_set = vec![false; self.lines.len()];
        for loc in &self.heading_locs {
            if loc.rendered_line < self.heading_line_set.len() {
                self.heading_line_set[loc.rendered_line] = true;
            }
        }
        self.rebuild_display();
        if !self.search_query.is_empty() {
            self.recompute_search();
        }
        self.clamp_offset();
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<bool, String> {
        if self.search_mode {
            self.handle_search_key(key);
            self.clamp_offset();
            return Ok(false);
        }
        if self.show_toc {
            self.handle_toc_key(key);
            self.clamp_offset();
            return Ok(false);
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(true),
            KeyCode::Char('?') => self.show_help = !self.show_help,
            KeyCode::Char('t') => {
                self.show_toc = !self.show_toc;
                self.show_help = false;
                if self.show_toc {
                    self.sync_toc_to_current_heading();
                    self.toc_filter_mode = false;
                    self.toc_filter_draft = self.toc_filter.clone();
                }
            }
            KeyCode::Char('/') => {
                self.search_mode = true;
                self.search_saved_query = self.search_query.clone();
                self.search_draft = self.search_query.clone();
                self.set_search_query_no_jump(&self.search_draft.clone());
                self.show_help = false;
                self.show_toc = false;
            }
            _ if self.show_help => {}
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
            KeyCode::Down | KeyCode::Char('j') => self.scroll_by(1),
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

    fn draw(&self, stdout: &mut io::Stdout) -> Result<(), String> {
        execute!(stdout, MoveTo(0, 0), Clear(ClearType::All))
            .map_err(|err| format!("draw: {err}"))?;
        if self.show_help {
            self.draw_help(stdout)?;
        } else if self.show_toc {
            self.draw_toc(stdout)?;
        } else {
            self.draw_main(stdout)?;
        }
        stdout.flush().map_err(|err| format!("flush: {err}"))
    }

    fn draw_main(&self, stdout: &mut io::Stdout) -> Result<(), String> {
        self.draw_bar(stdout, 0, &self.header_text(), true)?;
        let content_height = self.page_size();
        let start = min(self.offset, self.display.len());
        let end = min(start + content_height, self.display.len());
        let text_width = self.body_text_width();

        for row in 0..content_height {
            execute!(stdout, MoveTo(0, (row + 1) as u16))
                .map_err(|err| format!("draw row: {err}"))?;
            if start + row < end {
                let rendered_idx = self.display.at(start + row);
                let marker = self.marker(rendered_idx);
                print_colored(stdout, &format!(" {marker} "), Color::DarkGrey, None)?;
                let mut line = self.lines.get(rendered_idx).cloned().unwrap_or_default();
                line = fit_visible(&line, text_width);
                if self.is_heading_rendered_line(rendered_idx) {
                    execute!(
                        stdout,
                        SetBackgroundColor(Color::DarkGrey),
                        Print(line),
                        ResetColor
                    )
                    .map_err(|err| format!("draw heading: {err}"))?;
                } else {
                    execute!(stdout, Print(line)).map_err(|err| format!("draw body: {err}"))?;
                }
                execute!(stdout, Print(self.scrollbar_char(row, content_height)))
                    .map_err(|err| format!("draw scrollbar: {err}"))?;
            } else {
                execute!(
                    stdout,
                    Print("   "),
                    Print(" ".repeat(text_width)),
                    Print(" ")
                )
                .map_err(|err| format!("draw padding: {err}"))?;
            }
        }
        self.draw_bar(
            stdout,
            self.height.saturating_sub(1) as u16,
            &self.footer_text(),
            false,
        )
    }

    fn draw_help(&self, stdout: &mut io::Stdout) -> Result<(), String> {
        let lines = [
            "Keys",
            "",
            "  q / Esc        quit",
            "  j/k, Up/Down   scroll",
            "  PgUp/PgDn      page",
            "  u/d            half page",
            "  g/G            top / bottom",
            "  1-6 / 0        fold outline by heading level",
            "  [ / ]          previous/next heading",
            "  /              search (n/N to navigate, c to clear)",
            "  t              table of contents",
            "  / (in TOC)     filter headings",
            "  ?              toggle this help",
            "  mouse wheel    scroll",
        ];
        let top = self.center_top(lines.len());
        for (idx, line) in lines.iter().enumerate() {
            self.draw_centered(stdout, top + idx, line)?;
        }
        Ok(())
    }

    fn draw_toc(&self, stdout: &mut io::Stdout) -> Result<(), String> {
        let title = if self.headings.is_empty() {
            "TOC (no headings)"
        } else {
            "TOC"
        };
        let filtered = self.toc_filtered_headings();
        let filter = if self.toc_filter_mode {
            format!("/{}", self.toc_filter_draft)
        } else if self.toc_filter.trim().is_empty() {
            "/ filter".to_string()
        } else {
            format!(
                "/{} ({}/{})",
                self.toc_filter,
                filtered.len(),
                self.headings.len()
            )
        };
        let body_height = min(19, self.height.saturating_sub(5).max(3));
        let mut lines = vec![title.to_string(), filter, String::new()];

        if filtered.is_empty() {
            lines.push(if self.headings.is_empty() {
                "  (no headings found)".to_string()
            } else {
                "  (no matches)".to_string()
            });
        } else {
            let start = self.toc_idx.saturating_sub(body_height / 2);
            let start = min(start, filtered.len().saturating_sub(body_height));
            let end = min(filtered.len(), start + body_height);
            for (idx, h) in filtered[start..end].iter().enumerate() {
                let actual = start + idx;
                let prefix = if actual == self.toc_idx { "> " } else { "  " };
                let indent = "  ".repeat(h.level.saturating_sub(1).min(5));
                lines.push(format!("{prefix}{indent}{}", h.text));
            }
        }
        lines.push(String::new());
        lines.push(if self.toc_filter_mode {
            "type to filter  Enter apply  Esc cancel".to_string()
        } else {
            "j/k move  Enter jump  / filter  Esc close".to_string()
        });

        let top = self.center_top(lines.len());
        for (idx, line) in lines.iter().enumerate() {
            self.draw_centered(stdout, top + idx, line)?;
        }
        Ok(())
    }

    fn draw_bar(
        &self,
        stdout: &mut io::Stdout,
        row: u16,
        text: &str,
        header: bool,
    ) -> Result<(), String> {
        let bg = if header {
            Color::DarkBlue
        } else {
            Color::DarkGrey
        };
        execute!(
            stdout,
            MoveTo(0, row),
            SetBackgroundColor(bg),
            SetForegroundColor(Color::White),
            Print(fit_visible(text, self.width)),
            ResetColor
        )
        .map_err(|err| format!("draw bar: {err}"))
    }

    fn draw_centered(&self, stdout: &mut io::Stdout, row: usize, text: &str) -> Result<(), String> {
        if row >= self.height {
            return Ok(());
        }
        let width = visible_width(text);
        let col = self.width.saturating_sub(width) / 2;
        execute!(stdout, MoveTo(col as u16, row as u16), Print(text))
            .map_err(|err| format!("draw centered: {err}"))
    }

    fn header_text(&self) -> String {
        let pct = self.progress_percent();
        let right = if self.fold_level > 0 {
            format!("{pct:>3}%  H{}", self.fold_level)
        } else {
            format!("{pct:>3}%")
        };
        let mut left = self.title.trim().to_string();
        if left.is_empty() {
            left = "md".to_string();
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
                "/".to_string()
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
            "q quit  ? help  / search  t toc  [ ] section  1-6 fold 0 all".to_string()
        };
        join_left_right(&left, &meta, self.width)
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
                self.set_search_query_no_jump(&self.search_draft.clone());
            }
            _ => {}
        }
    }

    fn handle_toc_key(&mut self, key: KeyEvent) {
        if self.toc_filter_mode {
            self.handle_toc_filter_key(key);
            return;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('t') => self.show_toc = false,
            KeyCode::Char('/') => {
                self.toc_filter_mode = true;
                self.toc_filter_draft = self.toc_filter.clone();
            }
            KeyCode::Down | KeyCode::Char('j') => self.toc_idx = self.toc_idx.saturating_add(1),
            KeyCode::Up | KeyCode::Char('k') => self.toc_idx = self.toc_idx.saturating_sub(1),
            KeyCode::Home | KeyCode::Char('g') => self.toc_idx = 0,
            KeyCode::End | KeyCode::Char('G') => {
                self.toc_idx = self.toc_filtered_headings().len().saturating_sub(1)
            }
            KeyCode::Enter => {
                self.jump_to_toc_heading();
                self.show_toc = false;
            }
            _ => {}
        }
        let max_idx = self.toc_filtered_headings().len().saturating_sub(1);
        self.toc_idx = min(self.toc_idx, max_idx);
    }

    fn handle_toc_filter_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.toc_filter_mode = false;
                self.toc_filter_draft = self.toc_filter.clone();
            }
            KeyCode::Enter => {
                self.toc_filter_mode = false;
                self.toc_filter = self.toc_filter_draft.trim().to_string();
                self.toc_idx = 0;
            }
            KeyCode::Backspace => {
                self.toc_filter_draft.pop();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.toc_filter_draft.clear();
            }
            KeyCode::Char(ch) => self.toc_filter_draft.push(ch),
            _ => {}
        }
        let max_idx = self.toc_filtered_headings().len().saturating_sub(1);
        self.toc_idx = min(self.toc_idx, max_idx);
    }

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
        let idx = self
            .search_matches
            .iter()
            .position(|line| *line >= self.offset)
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
            return;
        }
        let len = self.search_matches.len() as isize;
        self.search_idx = ((self.search_idx as isize + delta).rem_euclid(len)) as usize;
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
        } else if !self.search_query.is_empty() && self.search_matches.contains(&rendered_idx) {
            '*'
        } else if self.is_heading_rendered_line(rendered_idx) {
            '#'
        } else {
            ' '
        }
    }

    fn jump_heading(&mut self, delta: isize) {
        let mut locs = self.heading_locs.clone();
        if self.fold_level > 0 {
            let filtered: Vec<_> = locs
                .iter()
                .filter(|loc| loc.heading.level <= self.fold_level)
                .cloned()
                .collect();
            if !filtered.is_empty() {
                locs = filtered;
            }
        }
        if locs.is_empty() {
            return;
        }
        let idx = current_heading_index(&locs, self.anchor_line()).unwrap_or(0);
        let next = (idx as isize + delta).clamp(0, locs.len() as isize - 1) as usize;
        self.set_offset_for_rendered_line(locs[next].rendered_line);
    }

    fn sync_toc_to_current_heading(&mut self) {
        let Some(current) = self.current_heading_md_line() else {
            return;
        };
        let filtered = self.toc_filtered_headings();
        if let Some(idx) = filtered.iter().position(|h| h.line == current) {
            self.toc_idx = idx;
        }
    }

    fn jump_to_toc_heading(&mut self) {
        let filtered = self.toc_filtered_headings();
        let Some(heading) = filtered.get(self.toc_idx) else {
            return;
        };
        if let Some(loc) = self
            .heading_locs
            .iter()
            .find(|loc| loc.heading.line == heading.line)
        {
            self.set_offset_for_rendered_line(loc.rendered_line);
        }
    }

    fn toc_filtered_headings(&self) -> Vec<Heading> {
        let active = if self.toc_filter_mode {
            self.toc_filter_draft.trim()
        } else {
            self.toc_filter.trim()
        };
        if active.is_empty() {
            return self.headings.clone();
        }
        let matcher = SearchMatcher::new(active);
        self.headings
            .iter()
            .filter(|h| matcher.contains(&h.text))
            .cloned()
            .collect()
    }

    fn rebuild_display(&mut self) {
        if self.lines.is_empty() {
            self.display = DisplayIndex::Identity(0);
            self.offset = 0;
            return;
        }
        if self.fold_level == 0 || self.heading_locs.is_empty() {
            self.display = DisplayIndex::Identity(self.lines.len());
            self.clamp_offset();
            return;
        }
        let idx: Vec<_> = self
            .heading_locs
            .iter()
            .filter(|loc| loc.heading.level <= self.fold_level)
            .map(|loc| loc.rendered_line)
            .collect();
        if idx.is_empty() {
            self.display = DisplayIndex::Identity(self.lines.len());
            self.clamp_offset();
            return;
        }
        let anchor = self.anchor_line();
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
            DisplayIndex::List(lines) => {
                let mut best = 0;
                for (idx, value) in lines.iter().enumerate() {
                    if *value <= line {
                        best = idx;
                    } else {
                        break;
                    }
                }
                best
            }
        }
    }

    fn set_offset_for_rendered_line(&mut self, line: usize) {
        self.offset = self.display_row_for_rendered_line(line);
        self.clamp_offset();
    }

    fn anchor_line(&self) -> usize {
        if self.display.len() == 0 {
            return 0;
        }
        self.display
            .at(min(self.offset + 1, self.display.len() - 1))
    }

    fn current_breadcrumb(&self) -> Option<String> {
        let idx = current_heading_index(&self.heading_locs, self.anchor_line())?;
        let chain = breadcrumb_for_index(&self.heading_locs, idx);
        if chain.is_empty() {
            None
        } else {
            Some(
                chain
                    .iter()
                    .map(|loc| loc.heading.text.trim())
                    .collect::<Vec<_>>()
                    .join(" > "),
            )
        }
    }

    fn current_heading_md_line(&self) -> Option<usize> {
        let idx = current_heading_index(&self.heading_locs, self.anchor_line())?;
        Some(self.heading_locs[idx].heading.line)
    }

    fn is_heading_rendered_line(&self, idx: usize) -> bool {
        self.heading_line_set.get(idx).copied().unwrap_or(false)
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
        if self.lines.is_empty() {
            return 0;
        }
        let doc_max_off = self.lines.len().saturating_sub(self.page_size());
        if doc_max_off == 0 {
            return 100;
        }
        let (top, _, _) = self.visible_doc_range();
        min(100, top * 100 / doc_max_off)
    }

    fn visible_doc_range(&self) -> (usize, usize, usize) {
        let total = self.lines.len();
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

    fn center_top(&self, block_height: usize) -> usize {
        self.height.saturating_sub(block_height) / 2
    }

    fn set_status(&mut self, message: &str) {
        self.status_message = message.to_string();
        self.status_until = Some(Instant::now() + Duration::from_millis(1500));
    }
}

pub fn parse_headings(markdown: &str) -> Vec<Heading> {
    let normalized = markdown.replace("\r\n", "\n");
    let mut headings = Vec::new();
    let mut in_fence = false;
    let mut fence = "";

    for (idx, raw) in normalized.lines().enumerate() {
        let trimmed = raw.trim();
        if is_fence_line(trimmed) {
            if !in_fence {
                in_fence = true;
                fence = &trimmed[..3];
            } else if trimmed.starts_with(fence) {
                in_fence = false;
                fence = "";
            }
            continue;
        }
        if in_fence {
            continue;
        }
        if let Some((level, text)) = parse_heading(trimmed) {
            headings.push(Heading {
                level,
                text: text.to_string(),
                line: idx,
            });
        }
    }
    headings
}

pub fn compute_heading_locs_from_rendered(
    plain: &[String],
    headings: &[Heading],
) -> Vec<HeadingLoc> {
    let mut cursor = 0;
    let mut out = Vec::new();
    for heading in headings {
        let target = normalize_text(&heading.text);
        if target.is_empty() {
            continue;
        }
        let mut found = None;
        for (idx, line) in plain.iter().enumerate().skip(cursor) {
            if normalize_text(line) == target || normalize_text(line).contains(&target) {
                found = Some(idx);
                break;
            }
        }
        if let Some(rendered_line) = found {
            out.push(HeadingLoc {
                heading: heading.clone(),
                rendered_line,
            });
            cursor = rendered_line + 1;
        }
    }
    out
}

pub fn current_heading_index(locs: &[HeadingLoc], anchor_line: usize) -> Option<usize> {
    let mut best = None;
    for (idx, loc) in locs.iter().enumerate() {
        if loc.rendered_line <= anchor_line {
            best = Some(idx);
        } else {
            break;
        }
    }
    best
}

pub fn breadcrumb_for_index(locs: &[HeadingLoc], idx: usize) -> Vec<HeadingLoc> {
    if idx >= locs.len() {
        return Vec::new();
    }
    let current = locs[idx].clone();
    let mut chain = vec![current.clone()];
    let mut level = current.heading.level;
    for loc in locs[..idx].iter().rev() {
        if loc.heading.level < level {
            chain.push(loc.clone());
            level = loc.heading.level;
        }
        if level <= 1 {
            break;
        }
    }
    chain.reverse();
    chain
}

struct SearchMatcher {
    query: String,
    lower: String,
    sensitive: bool,
}

impl SearchMatcher {
    fn new(query: &str) -> Self {
        let sensitive = query.chars().any(|ch| ch.is_ascii_uppercase());
        Self {
            query: query.to_string(),
            lower: query.to_ascii_lowercase(),
            sensitive,
        }
    }

    fn contains(&self, line: &str) -> bool {
        if self.sensitive {
            line.contains(&self.query)
        } else {
            line.to_ascii_lowercase().contains(&self.lower)
        }
    }
}

fn split_lines(input: &str) -> Vec<String> {
    let trimmed = input
        .replace("\r\n", "\n")
        .trim_end_matches('\n')
        .to_string();
    if trimmed.is_empty() {
        Vec::new()
    } else {
        trimmed.lines().map(ToString::to_string).collect()
    }
}

fn normalize_text(input: &str) -> String {
    input
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn join_left_right(left: &str, right: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let right_width = visible_width(right);
    let left_width = width.saturating_sub(right_width + 1);
    let left = fit_visible(left, left_width);
    let gap = width.saturating_sub(visible_width(&left) + right_width);
    format!("{left}{}{right}", " ".repeat(gap))
}

fn fit_visible(input: &str, width: usize) -> String {
    let plain_width = visible_width(input);
    if plain_width == width {
        return input.to_string();
    }
    if plain_width < width {
        return format!("{input}{}", " ".repeat(width - plain_width));
    }
    if width <= 3 {
        return ".".repeat(width);
    }
    let plain = strip_ansi(input);
    let mut out: String = plain.chars().take(width - 3).collect();
    out.push_str("...");
    out
}

fn visible_width(input: &str) -> usize {
    strip_ansi(input).chars().count()
}

fn print_colored(
    stdout: &mut io::Stdout,
    text: &str,
    fg: Color,
    bg: Option<Color>,
) -> Result<(), String> {
    execute!(stdout, SetForegroundColor(fg)).map_err(|err| format!("set color: {err}"))?;
    if let Some(bg) = bg {
        execute!(stdout, SetBackgroundColor(bg)).map_err(|err| format!("set bg: {err}"))?;
    }
    execute!(stdout, Print(text), ResetColor).map_err(|err| format!("print color: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_headings_ignores_fences() {
        let md = "# A\n\n```go\n# not a heading\n```\n\n## B\n";
        let headings = parse_headings(md);
        assert_eq!(headings.len(), 2);
        assert_eq!(headings[0].text, "A");
        assert_eq!(headings[0].level, 1);
        assert_eq!(headings[1].text, "B");
        assert_eq!(headings[1].level, 2);
    }

    #[test]
    fn compute_heading_locs_in_order() {
        let plain = vec![
            "Title".to_string(),
            "intro".to_string(),
            "Section A".to_string(),
            "body".to_string(),
            "Section B".to_string(),
        ];
        let headings = vec![
            Heading {
                level: 1,
                text: "Title".to_string(),
                line: 0,
            },
            Heading {
                level: 2,
                text: "Section A".to_string(),
                line: 10,
            },
            Heading {
                level: 2,
                text: "Section B".to_string(),
                line: 20,
            },
        ];
        let locs = compute_heading_locs_from_rendered(&plain, &headings);
        assert_eq!(locs.len(), 3);
        assert_eq!(locs[0].rendered_line, 0);
        assert_eq!(locs[1].rendered_line, 2);
        assert_eq!(locs[2].rendered_line, 4);
    }

    #[test]
    fn breadcrumb_parent_chain() {
        let locs = vec![
            loc(1, "H1", 0),
            loc(2, "H2", 5),
            loc(3, "H3", 10),
            loc(2, "H2b", 15),
        ];
        let chain = breadcrumb_for_index(&locs, 2);
        let texts: Vec<_> = chain.iter().map(|loc| loc.heading.text.as_str()).collect();
        assert_eq!(texts, vec!["H1", "H2", "H3"]);
    }

    fn loc(level: usize, text: &str, rendered_line: usize) -> HeadingLoc {
        HeadingLoc {
            heading: Heading {
                level,
                text: text.to_string(),
                line: rendered_line,
            },
            rendered_line,
        }
    }
}
