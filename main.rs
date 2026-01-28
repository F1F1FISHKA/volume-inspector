// main.rs
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Widget};
use ratatui::style::{Color, Style};
use crossterm::event::{self, Event, KeyCode};
use crossterm::terminal::{enable_raw_mode, disable_raw_mode};
use std::io;

#[derive(Debug, Clone)]
struct FileEntry {
    path: PathBuf,
    size: u64,
    file_type: String,
    children: Vec<FileEntry>,
}

fn get_file_type(path: &Path) -> String {
    match path.extension().and_then(|s| s.to_str()) {
        Some(ext) => ext.to_lowercase(),
        None => "other".to_string(),
    }
}

fn collect_files(dir: &Path) -> FileEntry {
    let mut root = FileEntry {
        path: dir.to_path_buf(),
        size: 0,
        file_type: "dir".to_string(),
        children: vec![],
    };
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            let child = collect_files(&path);
            root.size += child.size;
            root.children.push(child);
        } else if path.is_file() {
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let file_type = get_file_type(&path);
            root.size += size;
            root.children.push(FileEntry {
                path: path.clone(),
                size,
                file_type,
                children: vec![],
            });
        }
    }
    root
}

fn group_small_files(entries: &mut Vec<FileEntry>, min_size: u64) {
    let mut other = FileEntry {
        path: PathBuf::from("other"),
        size: 0,
        file_type: "other".to_string(),
        children: vec![],
    };
    entries.retain(|e| {
        if e.size < min_size && e.file_type != "dir" {
            other.size += e.size;
            other.children.push(e.clone());
            false
        } else {
            true
        }
    });
    if other.size > 0 {
        entries.push(other);
    }
}

fn file_type_color(file_type: &str) -> Color {
    match file_type {
        "rs" => Color::Green,
        "txt" => Color::Yellow,
        "md" => Color::Blue,
        "jpg" | "png" => Color::Magenta,
        "other" => Color::Gray,
        _ => Color::Cyan,
    }
}

fn draw_treemap<B: ratatui::backend::Backend>(
    f: &mut Frame<B>,
    area: Rect,
    entries: &[FileEntry],
    total_size: u64,
) {
    let mut x = area.x;
    let mut y = area.y;
    let mut width = area.width;
    let mut height = area.height;
    let mut horizontal = width > height;
    let mut offset = 0;
    for entry in entries {
        let ratio = entry.size as f64 / total_size as f64;
        let tile_size = if horizontal {
            (width as f64 * ratio).max(1.0) as u16
        } else {
            (height as f64 * ratio).max(1.0) as u16
        };
        let rect = if horizontal {
            Rect { x: x + offset, y, width: tile_size, height }
        } else {
            Rect { x, y: y + offset, width, height: tile_size }
        };
        let color = file_type_color(&entry.file_type);
        let block = Block::default()
            .borders(Borders::ALL)
            .style(Style::default().bg(color));
        f.render_widget(block, rect);
        if !entry.children.is_empty() && tile_size > 2 {
            draw_treemap(f, rect, &entry.children, entry.size);
        }
        offset += tile_size;
    }
}

fn main() -> Result<(), io::Error> {
    let args: Vec<String> = env::args().collect();
    let dir = if args.len() > 1 { &args[1] } else { "." };
    let root = collect_files(Path::new(dir));
    let mut entries = root.children.clone();
    let min_size = root.size / 50; // 2% threshold for 'other'
    group_small_files(&mut entries, min_size);

    enable_raw_mode()?;
    let mut terminal = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(io::stdout()))?;
    loop {
        terminal.draw(|f| {
            let size = f.size();
            let chunks = ratatui::layout::Layout::default()
                .direction(ratatui::layout::Direction::Vertical)
                .margin(1)
                .constraints([
                    ratatui::layout::Constraint::Min(0),
                    ratatui::layout::Constraint::Length(1),
                ])
                .split(size);
            draw_treemap(f, chunks[0], &entries, root.size);
            let status = format!("Dir: {} | Total size: {} bytes | q: quit", dir, root.size);
            let status_bar = Paragraph::new(status).style(Style::default().bg(Color::DarkGray).fg(Color::White));
            f.render_widget(status_bar, chunks[1]);
        })?;
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') {
                    break;
                }
            }
        }
    }
    disable_raw_mode()?;
    Ok(())
}
