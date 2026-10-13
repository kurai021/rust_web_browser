use super::*;
#[derive(Clone)]
struct Cell {
    node: usize,
    row: usize,
    col: usize,
    rows: usize,
    cols: usize,
}
struct Plan {
    rows: Vec<usize>,
    cells: Vec<Cell>,
    captions: Vec<usize>,
    groups: Vec<(usize, usize, usize)>,
    columns: usize,
}
impl Engine<'_> {
    pub(super) fn table_intrinsic(&mut self, index: usize, width: f32) -> f32 {
        let plan = self.table_plan(index);
        if plan.columns == 0 {
            return 0.0;
        }
        let mut columns = vec![0.0f32; plan.columns];
        for cell in plan.cells {
            let w = self.intrinsic_sizes(cell.node).1;
            let old = columns[cell.col..cell.col + cell.cols].iter().sum::<f32>();
            let extra = (w - old).max(0.0) / cell.cols as f32;
            for c in &mut columns[cell.col..cell.col + cell.cols] {
                *c += extra;
            }
        }
        let style = &self.tree.nodes[index].style;
        let spacing = if style.layout.border_collapse {
            0.0
        } else {
            style.layout.border_spacing[0]
                .resolve(self.ctx(style, width))
                .unwrap_or(0.0)
        };
        columns.iter().sum::<f32>() + spacing * (plan.columns + 1) as f32
    }
    fn table_plan(&self, index: usize) -> Plan {
        let mut plan = Plan {
            rows: Vec::new(),
            cells: Vec::new(),
            captions: Vec::new(),
            groups: Vec::new(),
            columns: 0,
        };
        let mut work = self.tree.nodes[index]
            .children
            .iter()
            .rev()
            .map(|&i| (i, None))
            .collect::<Vec<_>>();
        while let Some((i, group)) = work.pop() {
            match self.tree.nodes[i].style.layout.mode {
                Mode::Caption => plan.captions.push(i),
                Mode::Row => {
                    if plan.rows.len() >= 4096 {
                        break;
                    }
                    plan.rows.push(i);
                    if let Some(g) = group {
                        if let Some(entry) = plan.groups.iter_mut().find(|(id, _, _)| *id == g) {
                            entry.2 = plan.rows.len();
                        } else {
                            plan.groups.push((g, plan.rows.len() - 1, plan.rows.len()));
                        }
                    }
                }
                Mode::RowGroup => {
                    for &c in self.tree.nodes[i].children.iter().rev() {
                        work.push((c, Some(i)));
                    }
                }
                _ => {}
            }
        }
        let mut occupied = std::collections::HashSet::new();
        for (row, &r) in plan.rows.iter().enumerate() {
            let mut col = 0;
            for &c in &self.tree.nodes[r].children {
                if self.tree.nodes[c].style.layout.mode != Mode::Cell {
                    continue;
                }
                let [rs, cs] = self.tree.nodes[c].span;
                while col < 128 && (col..(col + cs).min(128)).any(|x| occupied.contains(&(row, x)))
                {
                    col += 1;
                }
                if col + cs > 128 {
                    break;
                }
                let rs = rs.min(plan.rows.len() - row);
                for y in row..row + rs {
                    for x in col..col + cs {
                        occupied.insert((y, x));
                    }
                }
                plan.cells.push(Cell {
                    node: c,
                    row,
                    col,
                    rows: rs,
                    cols: cs,
                });
                col += cs;
                plan.columns = plan.columns.max(col);
            }
        }
        plan
    }
    fn structural_box(&mut self, index: usize, rect: Rect, parent: usize) -> usize {
        let node = &self.tree.nodes[index];
        let i = self.result.boxes.len();
        self.result.boxes.push(LayoutBox {
            element: node.element,
            anonymous: false,
            style: node.style.clone(),
            border_box: rect,
            flow_box: rect,
            fixed: false,
            parent: Some(parent),
            formatting_context: self.result.boxes[parent].formatting_context,
            padding_box: rect,
            content_box: rect,
            border: Edges::default(),
            content: Vec::new(),
            overflow: Overflow::Visible,
            scroll_size: rect.size(),
            radius: 0.0,
            background_image: None,
        });
        i
    }
    pub(super) fn table(
        &mut self,
        index: usize,
        parent: usize,
        content: Rect,
        height: Option<f32>,
        ancestors: &[usize],
    ) -> f32 {
        let plan = self.table_plan(index);
        let node = &self.tree.nodes[index];
        if plan.columns == 0 {
            return self.inline(index, parent, content, ancestors);
        }
        let [sx, sy] = if node.style.layout.border_collapse {
            [0.0, 0.0]
        } else {
            node.style.layout.border_spacing.clone().map(|l| {
                l.resolve(self.ctx(&node.style, content.width))
                    .unwrap_or(0.0)
                    .max(0.0)
            })
        };
        let mut min = vec![0.0f32; plan.columns];
        let mut max = min.clone();
        for cell in &plan.cells {
            let (a, b) = self.intrinsic_sizes(cell.node);
            let existing = min[cell.col..cell.col + cell.cols].iter().sum::<f32>()
                + sx * (cell.cols - 1) as f32;
            let extra = (a - existing).max(0.0) / cell.cols as f32;
            let existing = max[cell.col..cell.col + cell.cols].iter().sum::<f32>()
                + sx * (cell.cols - 1) as f32;
            let extra_max = (b - existing).max(0.0) / cell.cols as f32;
            for i in cell.col..cell.col + cell.cols {
                min[i] += extra;
                max[i] += extra_max;
            }
        }
        let available = (content.width - sx * (plan.columns + 1) as f32).max(0.0);
        let mut widths = if node.style.layout.table_fixed {
            vec![available / plan.columns as f32; plan.columns]
        } else {
            min.clone()
        };
        if !node.style.layout.table_fixed {
            let free = (available - widths.iter().sum::<f32>()).max(0.0);
            let desired = max
                .iter()
                .zip(&min)
                .map(|(a, b)| (a - b).max(0.0))
                .sum::<f32>();
            for i in 0..widths.len() {
                widths[i] += if desired > 0.0 {
                    free * (max[i] - min[i]).max(0.0) / desired
                } else {
                    free / plan.columns as f32
                };
            }
        }
        let mut row_heights = plan
            .rows
            .iter()
            .map(|&r| {
                self.tree.nodes[r]
                    .style
                    .height
                    .resolve(self.ctx(&self.tree.nodes[r].style, height.unwrap_or(0.0)))
                    .unwrap_or(0.0)
            })
            .collect::<Vec<_>>();
        let mut natural = Vec::new();
        for cell in &plan.cells {
            let w = widths[cell.col..cell.col + cell.cols].iter().sum::<f32>()
                + sx * (cell.cols - 1) as f32;
            let n = &self.tree.nodes[cell.node];
            let (_, p, b) = self.edges(n, w);
            let h = self
                .measure(
                    cell.node,
                    w,
                    height,
                    UsedSize {
                        width: Some((w - p.horizontal() - b.horizontal()).max(0.0)),
                        height: None,
                    },
                )
                .0
                .height;
            natural.push(h);
            if cell.rows == 1 {
                row_heights[cell.row] = row_heights[cell.row].max(h);
            }
        }
        for (cell, &h) in plan.cells.iter().zip(&natural) {
            let old = row_heights[cell.row..cell.row + cell.rows]
                .iter()
                .sum::<f32>()
                + sy * (cell.rows - 1) as f32;
            let extra = (h - old).max(0.0) / cell.rows as f32;
            for row in &mut row_heights[cell.row..cell.row + cell.rows] {
                *row += extra;
            }
        }
        let mut y = content.y;
        for caption in plan.captions {
            let root = self.item(
                caption,
                content,
                y,
                height,
                ancestors,
                UsedSize {
                    width: Some(content.width),
                    height: None,
                },
            );
            y = self.result.boxes[root].border_box.bottom();
            self.result.boxes[parent].content.push(Content::Box(root));
        }
        let top = y + sy;
        let mut group_boxes = Vec::new();
        for &(group, start, end) in &plan.groups {
            let rect = Rect::new(
                content.x + sx,
                top + row_heights[..start].iter().sum::<f32>() + sy * start as f32,
                available,
                row_heights[start..end].iter().sum::<f32>()
                    + sy * end.saturating_sub(start + 1) as f32,
            );
            let root = self.structural_box(group, rect, parent);
            self.result.boxes[parent].content.push(Content::Box(root));
            group_boxes.push((start, end, root));
        }
        let mut row_boxes = Vec::new();
        for (r, &row) in plan.rows.iter().enumerate() {
            let owner = group_boxes
                .iter()
                .find(|(start, end, _)| r >= *start && r < *end)
                .map_or(parent, |(_, _, root)| *root);
            let rect = Rect::new(
                content.x + sx,
                top + row_heights[..r].iter().sum::<f32>() + sy * r as f32,
                available,
                row_heights[r],
            );
            let root = self.structural_box(row, rect, owner);
            self.result.boxes[owner].content.push(Content::Box(root));
            row_boxes.push(root);
        }
        for (cell, &natural) in plan.cells.iter().zip(&natural) {
            let w = widths[cell.col..cell.col + cell.cols].iter().sum::<f32>()
                + sx * (cell.cols - 1) as f32;
            let h = row_heights[cell.row..cell.row + cell.rows]
                .iter()
                .sum::<f32>()
                + sy * (cell.rows - 1) as f32;
            let n = &self.tree.nodes[cell.node];
            let (_, p, b) = self.edges(n, w);
            let mut path = ancestors.to_vec();
            let owner = row_boxes[cell.row];
            if let Some(g) = self.result.boxes[owner].parent {
                if g != parent {
                    path.push(g);
                }
            }
            path.push(owner);
            let x = content.x + sx + widths[..cell.col].iter().sum::<f32>() + sx * cell.col as f32;
            let y = top + row_heights[..cell.row].iter().sum::<f32>() + sy * cell.row as f32;
            let root = self.item(
                cell.node,
                Rect::new(x, y, w, h),
                y,
                Some(h),
                &path,
                UsedSize {
                    width: Some((w - p.horizontal() - b.horizontal()).max(0.0)),
                    height: Some((h - p.vertical() - b.vertical()).max(0.0)),
                },
            );
            let dy = match n.style.get_property_value("vertical-align").as_str() {
                "middle" => (h - natural) / 2.0,
                "bottom" => h - natural,
                _ => 0.0,
            };
            if dy > 0.0 {
                let old = self.result.boxes[root].clone();
                self.shift_box(root, 0.0, dy);
                let b = &mut self.result.boxes[root];
                b.border_box = old.border_box;
                b.padding_box = old.padding_box;
                b.content_box = old.content_box;
                b.flow_box = old.flow_box;
            }
            self.result.boxes[owner].content.push(Content::Box(root));
        }
        (top - content.y + row_heights.iter().sum::<f32>() + sy * plan.rows.len() as f32)
            .max(height.unwrap_or(0.0))
    }
}
