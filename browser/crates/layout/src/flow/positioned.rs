use super::*;
#[derive(Clone)]
pub(super) struct Pending {
    node: usize,
    parent: usize,
    static_point: (f32, f32),
    ancestors: Vec<usize>,
}
impl Engine<'_> {
    pub(super) fn defer_positioned(
        &mut self,
        node: usize,
        parent: usize,
        static_point: (f32, f32),
        ancestors: &[usize],
    ) {
        let mut path = ancestors.to_vec();
        if path.is_empty() {
            let mut p = Some(parent);
            while let Some(i) = p {
                path.push(i);
                p = self.result.boxes[i].parent;
            }
            path.reverse();
        }
        self.positioned.push(Pending {
            node,
            parent,
            static_point,
            ancestors: path,
        });
    }
    pub(super) fn layout_positioned(&mut self) {
        let mut cursor = 0;
        while cursor < self.positioned.len() && self.result.boxes.len() < 200_000 {
            let pending = self.positioned[cursor].clone();
            cursor += 1;
            let node = &self.tree.nodes[pending.node];
            let spec = &node.style.layout;
            let fixed = spec.position == Position::Fixed;
            let containing = if fixed {
                Rect::new(0.0, 0.0, self.viewport.width, self.viewport.height)
            } else {
                pending
                    .ancestors
                    .iter()
                    .rev()
                    .find(|&&i| {
                        !self.result.boxes[i].anonymous
                            && self.result.boxes[i].style.layout.position != Position::Static
                    })
                    .map_or(
                        Rect::new(0.0, 0.0, self.viewport.width, self.viewport.height),
                        |&i| self.result.boxes[i].padding_box,
                    )
            };
            let wc = self.ctx(&node.style, containing.width);
            let hc = self.ctx(&node.style, containing.height);
            let [top, right, bottom, left] = std::array::from_fn::<_, 4, _>(|i| {
                spec.inset[i].resolve(if i.is_multiple_of(2) { hc } else { wc })
            });
            let (m, p, b) = self.edges(node, containing.width);
            let inset = p.add_edges(b);
            let intrinsic = self.intrinsic_sizes(pending.node);
            let styled_width = node.style.width.resolve(wc).map(|w| {
                if node.style.border_box {
                    (w - inset.horizontal()).max(0.0)
                } else {
                    w
                }
            });
            let available = (containing.width
                - left.unwrap_or(0.0)
                - right.unwrap_or(0.0)
                - m.horizontal()
                - inset.horizontal())
            .max(0.0);
            let width = self.constrained_content(
                pending.node,
                styled_width.unwrap_or_else(|| {
                    if left.is_some() && right.is_some() {
                        available
                    } else {
                        (intrinsic.1 - m.horizontal() - inset.horizontal())
                            .min(available)
                            .max((intrinsic.0 - m.horizontal() - inset.horizontal()).min(available))
                    }
                }),
                containing.width,
                false,
            );
            let height = if top.is_some()
                && bottom.is_some()
                && matches!(node.style.height, SizeValue::Auto)
            {
                Some(
                    (containing.height
                        - top.unwrap_or(0.0)
                        - bottom.unwrap_or(0.0)
                        - inset.vertical()
                        - m.vertical())
                    .max(0.0),
                )
            } else {
                None
            };
            let start = self.result.boxes.len();
            let ancestors = if fixed {
                Vec::new()
            } else {
                pending.ancestors.clone()
            };
            let root = self.item(
                pending.node,
                containing,
                containing.y,
                Some(containing.height),
                &ancestors,
                UsedSize {
                    width: Some(width),
                    height,
                },
            );
            let old = self.result.boxes[root].border_box;
            let extra = (containing.width
                - left.unwrap_or(0.0)
                - right.unwrap_or(0.0)
                - old.width
                - m.horizontal())
            .max(0.0);
            let auto_left = matches!(node.style.margin[3], SizeValue::Auto);
            let auto_right = matches!(node.style.margin[1], SizeValue::Auto);
            let mx = if left.is_some() && right.is_some() && auto_left {
                if auto_right {
                    extra / 2.0
                } else {
                    extra
                }
            } else {
                m.left
            };
            let x = if let Some(l) = left {
                containing.x + l + mx
            } else if let Some(r) = right {
                containing.right() - r - old.width - m.right
            } else {
                pending.static_point.0 + m.left
            };
            let y = if let Some(t) = top {
                containing.y + t + m.top
            } else if let Some(v) = bottom {
                containing.bottom() - v - old.height - m.bottom
            } else {
                pending.static_point.1 + m.top
            };
            self.shift_box(root, x - old.x, y - old.y);
            if fixed || self.result.boxes[pending.parent].fixed {
                for b in &mut self.result.boxes[start..] {
                    b.fixed = true;
                }
                for h in &mut self.result.hits {
                    if h.ancestors.contains(&root) {
                        h.fixed = true;
                    }
                }
                for h in &mut self.result.targets {
                    if h.ancestors.contains(&root) {
                        h.fixed = true;
                    }
                }
            }
            self.result.boxes[root].parent = Some(pending.parent);
            self.result.boxes[pending.parent]
                .content
                .push(Content::Box(root));
        }
        if cursor < self.positioned.len() {
            self.result.stats.truncated = true;
        }
        // Overflow is recomputed after deferred children have their final boxes.
        for i in (0..self.result.boxes.len()).rev() {
            let b = &self.result.boxes[i];
            let mut right = b.padding_box.right();
            let mut bottom = b.padding_box.bottom();
            for child in &b.content {
                if let Content::Box(c) = child {
                    let c = &self.result.boxes[*c];
                    if c.fixed {
                        continue;
                    }
                    right = right.max(c.border_box.right());
                    bottom = bottom.max(c.border_box.bottom());
                    if c.overflow == Overflow::Visible {
                        right = right.max(c.padding_box.x + c.scroll_size.width);
                        bottom = bottom.max(c.padding_box.y + c.scroll_size.height);
                    }
                }
            }
            let b = &mut self.result.boxes[i];
            b.scroll_size.width = b.scroll_size.width.max(right - b.padding_box.x);
            b.scroll_size.height = b.scroll_size.height.max(bottom - b.padding_box.y);
        }
    }
    pub(super) fn clearance(&self, owner: usize, y: f32, clear: Clear) -> f32 {
        let owner = self.result.boxes[owner].formatting_context;
        self.float_regions.get(&owner).map_or(y, |regions| {
            regions
                .iter()
                .filter(|(_, side)| {
                    clear == Clear::Both
                        || clear == Clear::Left && *side == Float::Left
                        || clear == Clear::Right && *side == Float::Right
                })
                .map(|(r, _)| r.bottom())
                .fold(y, f32::max)
        })
    }
    pub(super) fn float_line(
        &self,
        owner: usize,
        content: Rect,
        y: f32,
        height: f32,
    ) -> (f32, f32) {
        let owner = self.result.boxes[owner].formatting_context;
        let mut left = content.x;
        let mut right = content.right();
        if let Some(regions) = self.float_regions.get(&owner) {
            for (r, side) in regions {
                if r.y < y + height && r.bottom() > y {
                    if *side == Float::Left {
                        left = left.max(r.right());
                    } else {
                        right = right.min(r.x);
                    }
                }
            }
        }
        (left, (right - left).max(0.0))
    }
    pub(super) fn place_float(
        &mut self,
        index: usize,
        parent: usize,
        content: Rect,
        y: f32,
        height: Option<f32>,
        ancestors: &[usize],
    ) {
        let node = &self.tree.nodes[index];
        let (m, p, b) = self.edges(node, content.width);
        let inset = p.add_edges(b);
        let size = self.intrinsic_sizes(index);
        let w = node
            .style
            .width
            .resolve(self.ctx(&node.style, content.width))
            .map(|w| {
                if node.style.border_box {
                    w - inset.horizontal()
                } else {
                    w
                }
            })
            .unwrap_or(
                (size.1 - m.horizontal() - inset.horizontal())
                    .min(content.width)
                    .max(0.0),
            );
        let root = self.item(
            index,
            content,
            0.0,
            height,
            ancestors,
            UsedSize {
                width: Some(w),
                height: None,
            },
        );
        let rect = self.result.boxes[root].border_box;
        let mut y = self.clearance(parent, y, node.style.layout.clear);
        for _ in 0..512 {
            let (_, space) = self.float_line(parent, content, y, rect.height + m.vertical());
            if space + 0.01 >= rect.width + m.horizontal() {
                break;
            }
            let ctx = self.result.boxes[parent].formatting_context;
            let next = self.float_regions.get(&ctx).and_then(|r| {
                r.iter()
                    .map(|(r, _)| r.bottom())
                    .filter(|&bottom| bottom > y)
                    .min_by(f32::total_cmp)
            });
            let Some(next) = next else {
                break;
            };
            y = next;
        }
        let (left, space) = self.float_line(parent, content, y, rect.height + m.vertical());
        let x = if node.style.layout.float == Float::Right {
            left + space - rect.width - m.right
        } else {
            left + m.left
        };
        self.shift_box(root, x - rect.x, y + m.top - rect.y);
        let margin_rect = Rect::new(
            x - m.left,
            y,
            rect.width + m.horizontal(),
            rect.height + m.vertical(),
        );
        let owner = self.result.boxes[parent].formatting_context;
        self.float_regions
            .entry(owner)
            .or_default()
            .push((margin_rect, node.style.layout.float));
        self.result.boxes[parent].content.push(Content::Box(root));
    }
}
