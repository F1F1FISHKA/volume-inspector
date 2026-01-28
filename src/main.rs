use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, MouseEventKind, EnableMouseCapture, DisableMouseCapture},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use std::fs;
use std::io::stdout;
use std::path::{Path, PathBuf};
use humansize::{SizeFormatter, DECIMAL};
use std::collections::HashMap;
use once_cell::sync::Lazy;
use seahash::hash;

#[derive(Parser)]
struct Args {
    #[arg(default_value = ".")]
    path: PathBuf,
}

#[derive(Clone)]
struct Node {
    name: String,
    size: u64,
    path: PathBuf,
    children: Vec<Node>,
    is_dir: bool,
}

impl Node {
    fn total_size(&self) -> u64 {
        self.size
    }
}

static COLOR_CACHE: Lazy<std::sync::Mutex<HashMap<String, Color>>> = 
    Lazy::new(|| std::sync::Mutex::new(HashMap::new()));

fn color_for_extension(ext: Option<&str>) -> Color {
    let ext = ext.unwrap_or("").to_lowercase();
    if ext.is_empty() {
        return Color::Rgb(150, 150, 150);
    }


    {
        let cache = COLOR_CACHE.lock().unwrap();
        if let Some(color) = cache.get(&ext) {
            return *color;
        }
    }


    let hash = hash(ext.as_bytes());
    

    let hue = ((hash >> 32) % 360) as f64;
    let saturation = 0.65 + ((hash >> 16) % 15) as f64 * 0.02;
    let lightness = 0.55 + ((hash >> 8) % 15) as f64 * 0.02;  
    
    let (r, g, b) = hsl_to_rgb(hue, saturation, lightness);
    

    {
        let mut cache = COLOR_CACHE.lock().unwrap();
        cache.insert(ext, Color::Rgb(r, g, b));
    }
    
    Color::Rgb(r, g, b)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    
    let (rp, gp, bp) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    
    let r = ((rp + m) * 255.0).clamp(0.0, 255.0) as u8;
    let g = ((gp + m) * 255.0).clamp(0.0, 255.0) as u8;
    let b = ((bp + m) * 255.0).clamp(0.0, 255.0) as u8;
    
    (r, g, b)
}

fn dynamic_color(node: &Node, total_size: u64, is_other: bool) -> Color {
    if total_size == 0 {
        return Color::DarkGray;
    }
    
    let norm = (node.size as f64 / total_size as f64).sqrt();
    let brightness = (90.0 + 165.0 * norm) as u8;

    if is_other {
        let gray = brightness.saturating_sub(30).clamp(60, 180);
        return Color::Rgb(gray, gray, gray);
    }

    if node.is_dir {
        let r = (brightness / 4) as u8;
        let g = (brightness * 2 / 3) as u8;
        let b = (brightness * 3 / 4 + 40) as u8;
        return Color::Rgb(r.clamp(30, 120), g.clamp(100, 220), b.clamp(120, 255));
    }

    let base = color_for_extension(node.path.extension().and_then(|s| s.to_str()));
    if let Color::Rgb(r, g, b) = base {
        let factor = 0.6 + norm * 0.8;
        let avg = (r as f64 + g as f64 + b as f64) / 3.0;
        
        let r_new = (r as f64 + (r as f64 - avg) * factor).clamp(60.0, 255.0) as u8;
        let g_new = (g as f64 + (g as f64 - avg) * factor).clamp(60.0, 255.0) as u8;
        let b_new = (b as f64 + (b as f64 - avg) * factor).clamp(60.0, 255.0) as u8;
        
        Color::Rgb(r_new, g_new, b_new)
    } else {
        Color::Rgb(brightness, brightness, brightness)
    }
}

fn build_tree(root: &Path) -> Result<Node> {
    let mut children = Vec::new();
    let mut total_size = 0u64;
    let mut file_count = 0;
    let mut file_total_size = 0u64;

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = entry.metadata()?;

        if metadata.is_symlink() {
            continue;
        }

        let name = path.file_name().map_or("".to_string(), |s| s.to_string_lossy().into_owned());

        if metadata.is_dir() {
            let child = build_tree(&path)?;
            total_size += child.total_size();
            children.push(child);
        } else if metadata.is_file() {
            let size = metadata.len();
            total_size += size;
            file_total_size += size;
            file_count += 1;
            children.push(Node {
                name,
                size,
                path,
                children: Vec::new(),
                is_dir: false,
            });
        }
    }

    children.sort_by_key(|c| std::cmp::Reverse(c.total_size()));

    let threshold = if file_count > 0 {
        let avg_size = file_total_size as f64 / file_count as f64;
        let count_factor = if file_count > 200 {
            0.001 
        } else if file_count > 50 {
            0.005 
        } else {
            0.01  
        };
        let size_based = total_size as f64 * count_factor;
        let avg_based = avg_size * 0.2; 
        
        size_based.max(avg_based).max(1024.0) as u64 
    } else {
        u64::MAX 
    };

    let mut other_size = 0u64;
    let mut filtered = Vec::new();

    for child in children {
        if !child.is_dir && child.size < threshold {
            other_size += child.size;
        } else {
            filtered.push(child);
        }
    }

    if other_size > 0 {
        filtered.push(Node {
            name: "Прочее".to_string(),
            size: other_size,
            path: root.to_path_buf(),
            children: Vec::new(),
            is_dir: false,
        });
    }

    let name = root.file_name().map_or("".to_string(), |s| s.to_string_lossy().into_owned());

    Ok(Node {
        name,
        size: total_size,
        path: root.to_path_buf(),
        children: filtered,
        is_dir: true,
    })
}

fn layout_tree<'a>(node: &'a Node, area: Rect, horizontal: bool) -> Vec<(Rect, &'a Node)> {
    if node.children.is_empty() || area.width < 3 || area.height < 3 {
        return vec![(area, node)];
    }

    let total = node.size as f64;
    let children: Vec<&'a Node> = node.children.iter()
        .filter(|c| c.size > 0)
        .collect();

    if children.is_empty() {
        return vec![(area, node)];
    }

    let primary_dim = if horizontal { area.width as f64 } else { area.height as f64 };
    let sizes: Vec<f64> = children.iter()
        .map(|c| (c.size as f64 / total) * primary_dim)
        .collect();

    let mut integer_sizes: Vec<u16> = sizes.iter().map(|&v| v.floor() as u16).collect();
    let allocated: u16 = integer_sizes.iter().sum();
    let remainder = primary_dim as u16 - allocated;

    let mut fractional: Vec<(usize, f64)> = sizes.iter()
        .enumerate()
        .map(|(i, &v)| (i, v.fract()))
        .collect();
    fractional.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    for i in 0..remainder as usize {
        if i < fractional.len() {
            integer_sizes[fractional[i].0] += 1;
        }
    }

    let mut result = Vec::new();
    let mut current_pos = if horizontal { area.x } else { area.y };
    let secondary_start = if horizontal { area.y } else { area.x };
    let secondary_size = if horizontal { area.height } else { area.width };

    for (i, &child) in children.iter().enumerate() {
        let mut size_primary = integer_sizes[i];
        if size_primary < 3 && primary_dim >= 3.0 {
            size_primary = 3;
        }
        if size_primary == 0 {
            continue;
        }

        let available = if horizontal {
            area.right().saturating_sub(current_pos)
        } else {
            area.bottom().saturating_sub(current_pos)
        };
        if size_primary > available {
            size_primary = available;
        }
        if size_primary < 3 {
            break;
        }

        let child_rect = if horizontal {
            Rect {
                x: current_pos,
                y: secondary_start,
                width: size_primary,
                height: secondary_size,
            }
        } else {
            Rect {
                x: secondary_start,
                y: current_pos,
                width: secondary_size,
                height: size_primary,
            }
        };

        result.extend(layout_tree(child, child_rect, !horizontal));
        current_pos += size_primary;
    }

    let remaining = if horizontal {
        area.right().saturating_sub(current_pos)
    } else {
        area.bottom().saturating_sub(current_pos)
    };
    if remaining > 0 && !result.is_empty() {
        let (last_rect, last_node) = result.pop().unwrap();
        let new_rect = if horizontal {
            Rect { width: last_rect.width + remaining, ..last_rect }
        } else {
            Rect { height: last_rect.height + remaining, ..last_rect }
        };
        result.push((new_rect, last_node));
    }

    result
}

fn clip_rect(rect: Rect, area: Rect) -> Option<Rect> {
    let x1 = rect.x.max(area.x);
    let y1 = rect.y.max(area.y);
    let x2 = (rect.x + rect.width).min(area.x + area.width);
    let y2 = (rect.y + rect.height).min(area.y + area.height);
    
    if x1 < x2 && y1 < y2 {
        Some(Rect {
            x: x1,
            y: y1,
            width: x2 - x1,
            height: y2 - y1,
        })
    } else {
        None
    }
}

struct App {
    root: Node,
    layout: Vec<(Rect, Node)>,
    layout_dirty: bool,
    last_area_size: (u16, u16),
    selected: Option<PathBuf>,
    current_dir: PathBuf,
    mouse_pos: (u16, u16),
    offset_x: u16,
    offset_y: u16,
    scroll_mode: bool,
}

impl App {
    fn new(root: Node) -> Self {
        let current_dir = root.path.clone();
        App {
            root,
            layout: Vec::new(),
            layout_dirty: true,
            last_area_size: (0, 0),
            selected: None,
            current_dir,
            mouse_pos: (0, 0),
            offset_x: 0,
            offset_y: 0,
            scroll_mode: false,
        }
    }

    fn find_node<'a>(&'a self, path: &Path) -> Option<&'a Node> {
        if self.root.path == path {
            return Some(&self.root);
        }
        fn recurse<'b>(node: &'b Node, path: &Path) -> Option<&'b Node> {
            if node.path == path {
                return Some(node);
            }
            for child in &node.children {
                if let Some(found) = recurse(child, path) {
                    return Some(found);
                }
            }
            None
        }
        recurse(&self.root, path)
    }

    fn get_node_at(&self, x: u16, y: u16) -> Option<&Node> {
        self.layout.iter()
            .find(|(rect, _)| {
                let rx = rect.x as i32 - self.offset_x as i32;
                let ry = rect.y as i32 - self.offset_y as i32;
                let rw = rect.width as i32;
                let rh = rect.height as i32;
                (x as i32) >= rx && (x as i32) < rx + rw && 
                (y as i32) >= ry && (y as i32) < ry + rh
            })
            .map(|(_, node)| node)
    }

    fn ensure_layout(&mut self, area: Rect) {
        let area_size = (area.width, area.height);
        let new_scroll_mode = area.width < 40 || area.height < 20;
        
        if self.layout_dirty 
            || self.last_area_size != area_size 
            || self.scroll_mode != new_scroll_mode 
        {
            self.recalculate_layout(area);
            self.last_area_size = area_size;
            self.scroll_mode = new_scroll_mode;
            self.layout_dirty = false;
        }
    }

    fn recalculate_layout(&mut self, area: Rect) {
        let current_node = self.find_node(&self.current_dir).unwrap_or(&self.root);
        let total_size = current_node.size;

        let layout_area = if self.scroll_mode {
            let node_count = current_node.children.len() as u16;
            let base_size = 200u16;
            let dynamic_size = (base_size + node_count * 5).min(5000);
            
            Rect {
                x: 0,
                y: 0,
                width: dynamic_size,
                height: dynamic_size,
            }
        } else {
            area
        };

        self.layout = layout_tree(current_node, layout_area, true)
            .into_iter()
            .map(|(r, n)| (r, n.clone()))
            .collect();
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let path = args.path.canonicalize()?;

    println!("Сканирую директорию...");
    let root = build_tree(&path)?;

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?.execute(EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(root);

    loop {
            //small optimizxcoDSAFNLKKLM'DBCVL;M
        let size = terminal.size()?;
        let area = Rect::new(0, 0, size.width, size.height);
        app.ensure_layout(area);
        
        terminal.draw(|f| ui(f, &mut app))?;

        match event::read()? {
            Event::Resize(_, _) => {
                app.layout_dirty = true;
            }
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Enter => {
                    if let Some(selected) = &app.selected {
                        if let Some(node) = app.find_node(selected) {
                            if node.is_dir && !node.children.is_empty() {
                                app.current_dir = node.path.clone();
                                app.offset_x = 0;
                                app.offset_y = 0;
                                app.layout_dirty = true;
                            }
                        }
                    }
                }
                KeyCode::Char('h') | KeyCode::Left => {
                    if app.scroll_mode {
                        app.offset_x = app.offset_x.saturating_sub(5);
                    } else if let Some(parent) = app.current_dir.parent() {
                        app.current_dir = parent.to_path_buf();
                        app.offset_x = 0;
                        app.offset_y = 0;
                        app.layout_dirty = true;
                    }
                }
                KeyCode::Char('l') | KeyCode::Right => {
                    if app.scroll_mode {
                        app.offset_x = app.offset_x.saturating_add(5);
                    }
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    if app.scroll_mode {
                        app.offset_y = app.offset_y.saturating_sub(3);
                    }
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    if app.scroll_mode {
                        app.offset_y = app.offset_y.saturating_add(3);
                    }
                }
                KeyCode::Char('H') => {
                    if app.scroll_mode {
                        app.offset_x = app.offset_x.saturating_sub(20);
                    }
                }
                KeyCode::Char('L') => {
                    if app.scroll_mode {
                        app.offset_x = app.offset_x.saturating_add(20);
                    }
                }
                KeyCode::Char('K') => {
                    if app.scroll_mode {
                        app.offset_y = app.offset_y.saturating_sub(10);
                    }
                }
                KeyCode::Char('J') => {
                    if app.scroll_mode {
                        app.offset_y = app.offset_y.saturating_add(10);
                    }
                }
                _ => {}
            },
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::Moved => {
                    app.mouse_pos = (mouse.column, mouse.row);
                    app.selected = app.get_node_at(mouse.column, mouse.row).map(|n| n.path.clone());
                }
                MouseEventKind::Down(_) => {
                    if let Some(node) = app.get_node_at(mouse.column, mouse.row) {
                        if node.is_dir && !node.children.is_empty() {
                            app.current_dir = node.path.clone();
                            app.offset_x = 0;
                            app.offset_y = 0;
                            app.layout_dirty = true;
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?.execute(DisableMouseCapture)?;
    Ok(())
}

fn ui(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(5)])
        .split(f.area());

    let main_area = chunks[0];
    let status_area = chunks[1];

    let current_node = app.find_node(&app.current_dir).unwrap_or(&app.root);
    let total_size = current_node.size;
    let current_name = current_node.name.clone();

    // another optimization
    for (rect, node) in &app.layout {

        let screen_x = rect.x as i32 - app.offset_x as i32;
        let screen_y = rect.y as i32 - app.offset_y as i32;
        let screen_right = screen_x + rect.width as i32;
        let screen_bottom = screen_y + rect.height as i32;
        
        let view_right = main_area.x as i32 + main_area.width as i32;
        let view_bottom = main_area.y as i32 + main_area.height as i32;
        
        if screen_right < main_area.x as i32 || screen_x >= view_right || 
           screen_bottom < main_area.y as i32 || screen_y >= view_bottom {
            continue; 
        }

        let mut draw_rect = *rect;
        if app.scroll_mode {
            draw_rect.x = draw_rect.x.saturating_sub(app.offset_x);
            draw_rect.y = draw_rect.y.saturating_sub(app.offset_y);
        }

        if let Some(clipped_rect) = clip_rect(draw_rect, main_area) {
            let is_selected = app.selected.as_ref().map_or(false, |p| p == &node.path);
            let is_other = node.name == "Прочее";
            let bg_color = dynamic_color(node, total_size, is_other);

            let border_style = if is_selected {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .border_type(ratatui::widgets::BorderType::Rounded);

            let text = if clipped_rect.width > 12 && clipped_rect.height > 4 {
                let size_str = if node.size < 1024 {
                    format!("{} байт", node.size)
                } else {
                    SizeFormatter::new(node.size, DECIMAL).to_string()
                };
                vec![
                    Line::from(node.name.clone()).centered(),
                    Line::from(size_str).centered(),
                ]
            } else {
                vec![]
            };

            let paragraph = Paragraph::new(text)
                .block(block)
                .style(Style::default().bg(bg_color).fg(Color::White))
                .alignment(ratatui::layout::Alignment::Center);

            f.render_widget(paragraph, clipped_rect);
        }
    }

    let mut status_lines = if let Some(selected_path) = &app.selected {
        if let Some(node) = app.get_node_at(app.mouse_pos.0, app.mouse_pos.1) {
            let name = selected_path.file_name().map_or("".to_string(), |s| s.to_string_lossy().into_owned());
            let size_str = if node.size < 1024 {
                format!("{} байт", node.size)
            } else {
                SizeFormatter::new(node.size, DECIMAL).to_string()
            };
            vec![
                Line::from(format!("Путь: {}", selected_path.display())),
                Line::from(format!("Имя: {} | Размер: {}", name, size_str)),
            ]
        } else {
            vec![
                Line::from(format!("Путь: {}", selected_path.display())),
                Line::from("Нет данных о файле".to_string()),
            ]
        }
    } else {
        let size_str = if total_size < 1024 {
            format!("{} байт", total_size)
        } else {
            SizeFormatter::new(total_size, DECIMAL).to_string()
        };
        vec![
            Line::from(format!("Текущая директория: {}", app.current_dir.display())),
            Line::from(format!("Имя: {} | Размер: {}", current_name, size_str)),
        ]
    };

    if app.scroll_mode {
        let scroll_hint = format!(
            "←/→/↑/↓: прокрутка | H/L: быстрая прокрутка | Смещение: {}, {}",
            app.offset_x, app.offset_y
        );
        status_lines.push(Line::from(scroll_hint).style(Style::default().fg(Color::Yellow)));
    }

    let status = Paragraph::new(status_lines)
        .style(Style::default().bg(Color::Rgb(20, 20, 30)).fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        );

    f.render_widget(status, status_area);
}