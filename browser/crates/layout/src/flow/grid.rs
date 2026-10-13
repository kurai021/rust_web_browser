use super::*;
use css::layout_values::{GridLine, Track, Tracks, MAX_TRACKS};
use std::collections::{HashMap, HashSet};
const MAX_ROWS: usize = 512;
struct Cell {
    node: usize,
    row: usize,
    col: usize,
    rows: usize,
    cols: usize,
}
fn boundary(line: &GridLine, count: usize) -> Option<usize> {
    match line {
        GridLine::Line(n) if *n > 0 => Some((*n as usize - 1).min(4095)),
        GridLine::Line(n) => Some((count as i32 + 1 + n).max(0) as usize),
        _ => None,
    }
}
fn interval(start: &GridLine, end: &GridLine, count: usize) -> (Option<usize>, usize) {
    let a = boundary(start, count);
    let b = boundary(end, count);
    let span = match (start, end) {
        (GridLine::Span(n), _) | (_, GridLine::Span(n)) => *n,
        _ => 1,
    };
    match (a, b) {
        (Some(a), Some(b)) => (Some(a.min(b)), a.abs_diff(b).clamp(1, MAX_TRACKS)),
        (Some(a), _) => (Some(a), span),
        (None, Some(b)) => (Some(b.saturating_sub(span)), span),
        _ => (None, span),
    }
}
impl Engine<'_> {
    fn track_min(&self, track: &Track, basis: f32, style: &ComputedStyle) -> f32 {
        match track {
            Track::Length(l) => l.resolve(self.ctx(style, basis)).unwrap_or(0.0).max(0.0),
            Track::MinMax(min, _) => self.track_min(min, basis, style),
            _ => 0.0,
        }
    }
    fn expanded_tracks(
        &self,
        tracks: &Tracks,
        basis: f32,
        gap: f32,
        items: usize,
        style: &ComputedStyle,
    ) -> Vec<Track> {
        let mut result = tracks.values.clone();
        if let Some((at, pattern, fit)) = &tracks.automatic {
            let fixed = result
                .iter()
                .map(|t| self.track_min(t, basis, style))
                .sum::<f32>();
            let unit = pattern
                .iter()
                .map(|t| self.track_min(t, basis, style))
                .sum::<f32>()
                + gap * pattern.len() as f32;
            let mut count = ((basis - fixed - gap * result.len() as f32 + gap) / unit.max(1.0))
                .floor()
                .max(1.0) as usize;
            count = count
                .min((MAX_TRACKS - result.len()) / pattern.len())
                .max(1);
            if *fit {
                count = count.min(items.div_ceil(pattern.len()).max(1));
            }
            let repeated = (0..count)
                .flat_map(|_| pattern.clone())
                .take(MAX_TRACKS - result.len())
                .collect::<Vec<_>>();
            result.splice(*at..*at, repeated);
        }
        result
    }
    #[allow(clippy::too_many_arguments)]
    fn track_sizes(
        &self,
        tracks: &[Track],
        basis: Option<f32>,
        gap: f32,
        min: &[f32],
        max: &[f32],
        style: &ComputedStyle,
        alignment: Align,
    ) -> Vec<f32> {
        let basis_value = basis.unwrap_or(0.0);
        let mut sizes = Vec::new();
        let mut fractions = Vec::new();
        let mut automatic = Vec::new();
        for (i, track) in tracks.iter().enumerate() {
            let (mut lower, upper) = match track {
                Track::MinMax(a, b) => (self.track_min(a, basis_value, style), &**b),
                Track::Fr(_) => (min[i], track),
                _ => (self.track_min(track, basis_value, style), track),
            };
            let size = match upper {
                Track::Length(l) => l
                    .resolve(self.ctx(style, basis_value))
                    .unwrap_or(max[i])
                    .max(lower),
                Track::Fr(fr) => {
                    fractions.push((i, *fr));
                    if basis.is_none() {
                        lower = lower.max(max[i]);
                    }
                    lower
                }
                Track::FitContent(l) => max[i]
                    .min(l.resolve(self.ctx(style, basis_value)).unwrap_or(max[i]))
                    .max(min[i])
                    .max(lower),
                _ => {
                    automatic.push(i);
                    max[i].max(min[i]).max(lower)
                }
            };
            sizes.push(size.max(0.0));
        }
        if let Some(basis) = basis {
            let floors = sizes.clone();
            let available = (basis - gap * tracks.len().saturating_sub(1) as f32).max(0.0);
            let mut frozen = vec![false; fractions.len()];
            for _ in 0..MAX_TRACKS {
                let fixed = sizes
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| {
                        !fractions
                            .iter()
                            .enumerate()
                            .any(|(j, (k, _))| !frozen[j] && k == i)
                    })
                    .map(|(_, v)| *v)
                    .sum::<f32>();
                let weight = fractions
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| !frozen[*j])
                    .map(|(_, (_, f))| *f)
                    .sum::<f32>()
                    .max(1.0);
                let fraction = (available - fixed).max(0.0) / weight;
                let mut changed = false;
                for (j, &(i, f)) in fractions.iter().enumerate() {
                    if frozen[j] {
                        continue;
                    }
                    let n = f * fraction;
                    if n + 0.01 < floors[i] {
                        frozen[j] = true;
                        changed = true;
                    } else {
                        sizes[i] = n;
                    }
                }
                if !changed {
                    break;
                }
            }
            let extra = (available - sizes.iter().sum::<f32>()).max(0.0);
            if fractions.is_empty()
                && matches!(alignment, Align::Normal | Align::Stretch)
                && !automatic.is_empty()
            {
                for i in automatic {
                    sizes[i] += extra
                        / tracks
                            .iter()
                            .filter(|t| matches!(t, Track::Auto))
                            .count()
                            .max(1) as f32;
                }
            }
        }
        sizes.into_iter().map(finite).collect()
    }
    pub(super) fn grid(
        &mut self,
        index: usize,
        parent: usize,
        content: Rect,
        height: Option<f32>,
        ancestors: &[usize],
    ) -> f32 {
        let node = &self.tree.nodes[index];
        let style = &node.style.layout;
        let row_gap = style.gap[0]
            .resolve(self.ctx(&node.style, height.unwrap_or(0.0)))
            .unwrap_or(0.0)
            .max(0.0);
        let col_gap = style.gap[1]
            .resolve(self.ctx(&node.style, content.width))
            .unwrap_or(0.0)
            .max(0.0);
        let mut children = node.children.clone();
        children.sort_by_key(|&c| self.tree.nodes[c].style.layout.order);
        let mut columns = self.expanded_tracks(
            &style.columns,
            content.width,
            col_gap,
            children.len(),
            &node.style,
        );
        let mut rows = self.expanded_tracks(
            &style.rows,
            height.unwrap_or(0.0),
            row_gap,
            children.len(),
            &node.style,
        );
        let mut areas = HashMap::<String, (usize, usize, usize, usize)>::new();
        for (r, row) in style.areas.iter().enumerate() {
            for (c, name) in row.iter().enumerate() {
                if name == "." {
                    continue;
                }
                areas
                    .entry(name.clone())
                    .and_modify(|a| {
                        a.2 = a.2.max(r + 1);
                        a.3 = a.3.max(c + 1);
                    })
                    .or_insert((r, c, r + 1, c + 1));
            }
        }
        let mut ncols = columns
            .len()
            .max(style.areas.first().map_or(0, Vec::len))
            .max(1);
        let mut nrows = rows.len().max(style.areas.len()).max(1);
        let mut pending = Vec::new();
        for child in children {
            let n = &self.tree.nodes[child];
            if n.out_of_flow() {
                self.defer_positioned(child, parent, (content.x, content.y), ancestors);
                continue;
            }
            let area = n
                .style
                .layout
                .area
                .as_ref()
                .and_then(|name| areas.get(name));
            let ((row, rs), (col, cs)) = if let Some(&(r, c, re, ce)) = area {
                ((Some(r), re - r), (Some(c), ce - c))
            } else {
                let p = &n.style.layout.placement;
                (interval(&p[0], &p[2], nrows), interval(&p[1], &p[3], ncols))
            };
            if let Some(c) = col {
                ncols = ncols.max((c + cs).min(MAX_TRACKS));
            }
            if let Some(r) = row {
                nrows = nrows.max((r + rs).min(MAX_ROWS));
            }
            pending.push((child, row, col, rs, cs));
        }
        let mut occupied = HashSet::new();
        let mut cells = Vec::new();
        // Definite areas occupy their cells before any auto-placement scan.
        for &(child, row, col, rs, cs) in &pending {
            if let (Some(r), Some(c)) = (row, col) {
                if r + rs > MAX_ROWS || c + cs > MAX_TRACKS {
                    self.result.stats.truncated = true;
                    continue;
                }
                cells.push(Cell {
                    node: child,
                    row: r,
                    col: c,
                    rows: rs,
                    cols: cs,
                });
                for y in r..r + rs {
                    for x in c..c + cs {
                        occupied.insert((y, x));
                    }
                }
            }
        }
        let mut cursor = (0usize, 0usize);
        for (child, row, col, rs, cs) in pending {
            if row.is_some() && col.is_some() {
                continue;
            }
            let mut point = if style.dense { (0, 0) } else { cursor };
            if let Some(r) = row {
                point = (r, 0);
            }
            if let Some(c) = col {
                point = (0, c);
            }
            let mut found = None;
            for _ in 0..MAX_ROWS * MAX_TRACKS {
                if point.0 + rs <= MAX_ROWS
                    && point.1 + cs <= MAX_TRACKS
                    && (style.auto_column_flow || col.is_some() || point.1 + cs <= ncols)
                    && (!style.auto_column_flow || row.is_some() || point.0 + rs <= nrows)
                    && (point.0..point.0 + rs)
                        .all(|r| (point.1..point.1 + cs).all(|c| !occupied.contains(&(r, c))))
                {
                    found = Some(point);
                    break;
                }
                if row.is_some() {
                    point.1 += 1;
                } else if col.is_some() {
                    point.0 += 1;
                } else if style.auto_column_flow {
                    point.0 += 1;
                    if point.0 + rs > nrows {
                        point.0 = 0;
                        point.1 += 1;
                    }
                } else {
                    point.1 += 1;
                    if point.1 + cs > ncols {
                        point.1 = 0;
                        point.0 += 1;
                    }
                }
                if point.0 >= MAX_ROWS || point.1 >= MAX_TRACKS {
                    break;
                }
            }
            let Some((r, c)) = found else {
                self.result.stats.truncated = true;
                continue;
            };
            nrows = nrows.max(r + rs);
            ncols = ncols.max(c + cs);
            cursor = (r, c + cs);
            if style.auto_column_flow {
                cursor = (r + rs, c);
            }
            for y in r..r + rs {
                for x in c..c + cs {
                    occupied.insert((y, x));
                }
            }
            cells.push(Cell {
                node: child,
                row: r,
                col: c,
                rows: rs,
                cols: cs,
            });
        }
        let auto_col = if style.auto_columns.values.is_empty() {
            vec![Track::Auto]
        } else {
            style.auto_columns.values.clone()
        };
        let auto_row = if style.auto_rows.values.is_empty() {
            vec![Track::Auto]
        } else {
            style.auto_rows.values.clone()
        };
        while columns.len() < ncols {
            columns.push(
                auto_col
                    [(columns.len().saturating_sub(style.columns.values.len())) % auto_col.len()]
                .clone(),
            );
        }
        while rows.len() < nrows {
            rows.push(
                auto_row[(rows.len().saturating_sub(style.rows.values.len())) % auto_row.len()]
                    .clone(),
            );
        }
        let mut min = vec![0.0f32; ncols];
        let mut max = min.clone();
        for cell in &cells {
            let (a, b) = self.intrinsic_sizes(cell.node);
            let old = min[cell.col..cell.col + cell.cols].iter().sum::<f32>()
                + col_gap * (cell.cols - 1) as f32;
            let extra = (a - old).max(0.0) / cell.cols as f32;
            let old_max = max[cell.col..cell.col + cell.cols].iter().sum::<f32>()
                + col_gap * (cell.cols - 1) as f32;
            let extra_max = (b - old_max).max(0.0) / cell.cols as f32;
            for i in cell.col..cell.col + cell.cols {
                min[i] += extra;
                max[i] += extra_max;
            }
        }
        let widths = self.track_sizes(
            &columns,
            Some(content.width),
            col_gap,
            &min,
            &max,
            &node.style,
            style.justify_content,
        );
        let mut row_min = vec![0.0f32; nrows];
        for cell in &cells {
            let area_width = widths[cell.col..cell.col + cell.cols].iter().sum::<f32>()
                + col_gap * (cell.cols - 1) as f32;
            let n = &self.tree.nodes[cell.node];
            let (m, p, b) = self.edges(n, area_width);
            let inset = p.add_edges(b);
            let width = if matches!(n.style.width, SizeValue::Auto) {
                Some((area_width - m.horizontal() - inset.horizontal()).max(0.0))
            } else {
                None
            };
            let h = self
                .measure(
                    cell.node,
                    area_width,
                    height,
                    UsedSize {
                        width,
                        height: None,
                    },
                )
                .0
                .height
                + m.vertical();
            let old = row_min[cell.row..cell.row + cell.rows].iter().sum::<f32>()
                + row_gap * (cell.rows - 1) as f32;
            let extra = (h - old).max(0.0) / cell.rows as f32;
            for size in &mut row_min[cell.row..cell.row + cell.rows] {
                *size += extra;
            }
        }
        let heights = self.track_sizes(
            &rows,
            height,
            row_gap,
            &row_min,
            &row_min,
            &node.style,
            style.align_content,
        );
        let natural = heights.iter().sum::<f32>() + row_gap * nrows.saturating_sub(1) as f32;
        let (xoff, xspace) = super::flex::distribute(
            style.justify_content,
            content.width - widths.iter().sum::<f32>() - col_gap * ncols.saturating_sub(1) as f32,
            ncols,
        );
        let (yoff, yspace) = super::flex::distribute(
            style.align_content,
            height.unwrap_or(natural) - natural,
            nrows,
        );
        cells.sort_by_key(|c| (self.tree.nodes[c.node].style.layout.order, c.node));
        for cell in cells {
            let n = &self.tree.nodes[cell.node];
            let w = widths[cell.col..cell.col + cell.cols].iter().sum::<f32>()
                + (col_gap + xspace) * (cell.cols - 1) as f32;
            let h = heights[cell.row..cell.row + cell.rows].iter().sum::<f32>()
                + (row_gap + yspace) * (cell.rows - 1) as f32;
            let (m, p, b) = self.edges(n, w);
            let inset = p.add_edges(b);
            let align = if n.style.layout.align_self == Align::Auto {
                style.align_items
            } else {
                n.style.layout.align_self
            };
            let justify = if n.style.layout.justify_self == Align::Auto {
                style.justify_items
            } else {
                n.style.layout.justify_self
            };
            let width = if matches!(n.style.width, SizeValue::Auto) {
                Some(if matches!(justify, Align::Normal | Align::Stretch) {
                    (w - m.horizontal() - inset.horizontal()).max(0.0)
                } else {
                    (self.intrinsic_width(cell.node) - m.horizontal() - inset.horizontal())
                        .min(w)
                        .max(0.0)
                })
            } else {
                None
            };
            let force = UsedSize {
                width,
                height: if matches!(align, Align::Normal | Align::Stretch)
                    && matches!(n.style.height, SizeValue::Auto)
                {
                    Some((h - m.vertical() - inset.vertical()).max(0.0))
                } else {
                    None
                },
            };
            let root = self.item(
                cell.node,
                Rect::new(content.x, content.y, w, h),
                content.y,
                Some(h),
                ancestors,
                force,
            );
            let rect = self.result.boxes[root].border_box;
            let mut dx = match justify {
                Align::End => (w - rect.width - m.horizontal()).max(0.0),
                Align::Center => (w - rect.width - m.horizontal()) / 2.0,
                _ => 0.0,
            };
            let mut dy = match align {
                Align::End => (h - rect.height - m.vertical()).max(0.0),
                Align::Center => (h - rect.height - m.vertical()) / 2.0,
                _ => 0.0,
            };
            let auto_x = usize::from(matches!(n.style.margin[3], SizeValue::Auto))
                + usize::from(matches!(n.style.margin[1], SizeValue::Auto));
            if auto_x > 0 && matches!(n.style.margin[3], SizeValue::Auto) {
                dx = (w - rect.width - m.horizontal()).max(0.0) / auto_x as f32;
            }
            let auto_y = usize::from(matches!(n.style.margin[0], SizeValue::Auto))
                + usize::from(matches!(n.style.margin[2], SizeValue::Auto));
            if auto_y > 0 && matches!(n.style.margin[0], SizeValue::Auto) {
                dy = (h - rect.height - m.vertical()).max(0.0) / auto_y as f32;
            }
            let x = content.x
                + xoff
                + widths[..cell.col].iter().sum::<f32>()
                + (col_gap + xspace) * cell.col as f32
                + m.left
                + dx;
            let y = content.y
                + yoff
                + heights[..cell.row].iter().sum::<f32>()
                + (row_gap + yspace) * cell.row as f32
                + m.top
                + dy;
            self.shift_box(root, x - rect.x, y - rect.y);
            self.result.boxes[parent].content.push(Content::Box(root));
        }
        height.unwrap_or(natural)
    }
}
