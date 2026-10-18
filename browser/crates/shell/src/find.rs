//! Find rendered text, using the shaper's UTF-8 clusters rather than byte widths.
use layout::{Content, LayoutResult, Rect};
use unicode_normalization::UnicodeNormalization;
#[derive(Clone, Debug)]
pub struct Part {
    pub rect: Rect,
    pub box_index: usize,
}
#[derive(Clone, Debug)]
pub struct Match {
    pub parts: Vec<Part>,
}
struct Character {
    start: usize,
    end: usize,
    part: Option<Part>,
}
fn union(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    Rect::new(
        x,
        y,
        (a.x + a.width).max(b.x + b.width) - x,
        (a.y + a.height).max(b.y + b.height) - y,
    )
}
pub fn search(layout: &LayoutResult, query: &str) -> Vec<Match> {
    let query = query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .nfd()
        .collect::<String>();
    if query.is_empty() || query.len() > 512 {
        return Vec::new();
    }
    let mut text = String::new();
    let mut characters: Vec<Character> = Vec::new();
    let mut previous = None::<Rect>;
    let mut work: Vec<(usize, usize)> = layout.roots.iter().rev().map(|&i| (i, 0)).collect();
    let mut visited = std::collections::HashSet::new();
    while let Some((index, content)) = work.pop() {
        let Some(b) = layout.boxes.get(index) else {
            continue;
        };
        if content == 0 && !visited.insert(index) {
            continue;
        }
        let Some(item) = b.content.get(content) else {
            continue;
        };
        work.push((index, content + 1));
        match item {
            Content::Box(child) => work.push((*child, 0)),
            Content::Text(run) if run.glyphs.iter().any(|g| g.color.a > 0) => {
                let separated = previous.is_some_and(|r| {
                    (run.rect.y - r.y).abs() > run.rect.height.max(1.0) * 0.6
                        || run.rect.x > r.x + r.width + 2.0
                });
                if separated && !text.ends_with(' ') && !text.is_empty() {
                    let start = text.len();
                    text.push(' ');
                    characters.push(Character {
                        start,
                        end: text.len(),
                        part: None,
                    });
                }
                let first = characters.len();
                let mut original = Vec::new();
                for (start, ch) in run.text.char_indices() {
                    let end = start + ch.len_utf8();
                    let begin = text.len();
                    if ch.is_whitespace() {
                        if !text.ends_with(' ') {
                            text.push(' ');
                        }
                    } else {
                        for c in ch.to_lowercase().nfd() {
                            text.push(c);
                        }
                    }
                    original.push((start, end));
                    characters.push(Character {
                        start: begin,
                        end: text.len(),
                        part: None,
                    });
                }
                for glyph in &run.glyphs {
                    let lo = original.partition_point(|(_, end)| *end <= glyph.start);
                    for (i, _) in original
                        .iter()
                        .enumerate()
                        .skip(lo)
                        .take_while(|(_, (start, _))| *start < glyph.end)
                    {
                        let rect = Rect::new(
                            glyph.position.0,
                            run.rect.y,
                            glyph.advance.max(1.0),
                            run.rect.height.max(1.0),
                        );
                        let c = &mut characters[first + i];
                        c.part = Some(Part {
                            rect: c.part.as_ref().map_or(rect, |p| union(p.rect, rect)),
                            box_index: index,
                        });
                    }
                }
                previous = Some(run.rect);
            }
            _ => {}
        }
        if text.len() > 2 * 1024 * 1024 {
            break;
        }
    }
    text.match_indices(&query)
        .take(5000)
        .map(|(start, needle)| {
            let end = start + needle.len();
            let lo = characters.partition_point(|c| c.end <= start);
            let mut parts: Vec<Part> = Vec::new();
            for c in characters.iter().skip(lo).take_while(|c| c.start < end) {
                if let Some(part) = &c.part {
                    if let Some(last) = parts.last_mut().filter(|p| {
                        p.box_index == part.box_index && (p.rect.y - part.rect.y).abs() < 1.0
                    }) {
                        last.rect = union(last.rect, part.rect);
                    } else {
                        parts.push(part.clone());
                    }
                }
            }
            Match { parts }
        })
        .filter(|m| !m.parts.is_empty())
        .collect()
}

pub struct Ready {
    pub ticket: u64,
    pub matches: Vec<Match>,
}
struct Request {
    ticket: u64,
    layout: std::sync::Arc<LayoutResult>,
    query: String,
}
pub struct Worker {
    requests: Option<std::sync::mpsc::Sender<Request>>,
    ready: std::sync::mpsc::Receiver<Ready>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Worker {
    pub fn spawn(wake: std::sync::Arc<dyn Fn() + Send + Sync>) -> std::io::Result<Self> {
        let (tx, rx) = std::sync::mpsc::channel::<Request>();
        let (done, ready) = std::sync::mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("find-worker".into())
            .spawn(move || {
                while let Ok(mut request) = rx.recv() {
                    while let Ok(newer) = rx.try_recv() {
                        request = newer;
                    }
                    let matches = search(&request.layout, &request.query);
                    if done
                        .send(Ready {
                            ticket: request.ticket,
                            matches,
                        })
                        .is_err()
                    {
                        break;
                    }
                    wake();
                }
            })?;
        Ok(Self {
            requests: Some(tx),
            ready,
            worker: Some(worker),
        })
    }
    pub fn request(&self, ticket: u64, layout: std::sync::Arc<LayoutResult>, query: String) {
        if let Some(sender) = &self.requests {
            let _ = sender.send(Request {
                ticket,
                layout,
                query,
            });
        }
    }
    pub fn drain(&self) -> Vec<Ready> {
        self.ready.try_iter().collect()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.requests.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
