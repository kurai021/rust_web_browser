use super::*;
struct Item {
    node: usize,
    base: f32,
    size: f32,
    min: f32,
    max: f32,
    grow: f32,
    shrink: f32,
    margin: Edges,
    inset: Edges,
    cross: f32,
    baseline: f32,
}
pub(super) fn distribute(align: Align, space: f32, count: usize) -> (f32, f32) {
    if count == 0 {
        return (0.0, 0.0);
    }
    let space = space.max(0.0);
    match align {
        Align::End => (space, 0.0),
        Align::Center => (space / 2.0, 0.0),
        Align::SpaceBetween if count > 1 => (0.0, space / (count - 1) as f32),
        Align::SpaceAround => (space / count as f32 / 2.0, space / count as f32),
        Align::SpaceEvenly => (space / (count + 1) as f32, space / (count + 1) as f32),
        _ => (0.0, 0.0),
    }
}
impl Engine<'_> {
    pub(super) fn flex(
        &mut self,
        index: usize,
        parent: usize,
        content: Rect,
        height: Option<f32>,
        ancestors: &[usize],
    ) -> f32 {
        let node = &self.tree.nodes[index];
        let style = &node.style.layout;
        let column = style.column;
        let main_definite = !column || height.is_some();
        let mut main = if column {
            height.unwrap_or(0.0)
        } else {
            content.width
        };
        let gap = style.gap[usize::from(!column)]
            .resolve(self.ctx(&node.style, main))
            .unwrap_or(0.0)
            .max(0.0);
        let cross_gap = style.gap[usize::from(column)]
            .resolve(self.ctx(&node.style, content.width))
            .unwrap_or(0.0)
            .max(0.0);
        let mut children = node.children.clone();
        children.sort_by_key(|&i| self.tree.nodes[i].style.layout.order);
        let mut items = Vec::new();
        for child in children {
            let n = &self.tree.nodes[child];
            if n.out_of_flow() {
                self.defer_positioned(child, parent, (content.x, content.y), ancestors);
                continue;
            }
            let (margin, padding, border) = self.edges(n, content.width);
            let inset = padding.add_edges(border);
            let axis_inset = if column {
                inset.vertical()
            } else {
                inset.horizontal()
            };
            let ctx = self.ctx(
                &n.style,
                if column {
                    height.unwrap_or(0.0)
                } else {
                    content.width
                },
            );
            let intrinsic = self.intrinsic_sizes(child);
            let dimension = if column {
                &n.style.height
            } else {
                &n.style.width
            };
            let base = n
                .style
                .layout
                .basis
                .resolve(ctx)
                .or_else(|| {
                    dimension.resolve(ctx).map(|v| {
                        if n.style.border_box {
                            v - axis_inset
                        } else {
                            v
                        }
                    })
                })
                .unwrap_or_else(|| {
                    if column {
                        self.measure(child, content.width, height, UsedSize::default())
                            .0
                            .height
                            - axis_inset
                    } else {
                        intrinsic.1 - margin.horizontal() - axis_inset
                    }
                })
                .max(0.0);
            let min_value = if column {
                &n.style.min_height
            } else {
                &n.style.min_width
            };
            let max_value = if column {
                &n.style.max_height
            } else {
                &n.style.max_width
            };
            let adjustment = |v: f32| {
                if n.style.border_box {
                    (v - axis_inset).max(0.0)
                } else {
                    v.max(0.0)
                }
            };
            let min = min_value.resolve(ctx).map(adjustment).unwrap_or_else(|| {
                if !column && overflow(&n.style) == Overflow::Visible {
                    (intrinsic.0 - margin.horizontal() - axis_inset)
                        .max(0.0)
                        .min(base)
                } else {
                    0.0
                }
            });
            let max = max_value
                .resolve(ctx)
                .map(adjustment)
                .unwrap_or(MAX_EXTENT)
                .max(min);
            items.push(Item {
                node: child,
                base,
                size: base.clamp(min, max),
                min,
                max,
                grow: n.style.layout.grow,
                shrink: n.style.layout.shrink,
                margin,
                inset,
                cross: 0.0,
                baseline: 0.0,
            });
        }
        if !main_definite {
            main = items
                .iter()
                .map(|i| i.size + i.inset.vertical() + i.margin.vertical())
                .sum::<f32>()
                + gap * (items.len().saturating_sub(1)) as f32;
        }
        let outer = |i: &Item| {
            i.size
                + if column {
                    i.margin.vertical() + i.inset.vertical()
                } else {
                    i.margin.horizontal() + i.inset.horizontal()
                }
        };
        let mut lines: Vec<Vec<usize>> = Vec::new();
        let mut line = Vec::new();
        let mut used = 0.0;
        for (i, item) in items.iter().enumerate() {
            let addition = outer(item) + if line.is_empty() { 0.0 } else { gap };
            if style.wrap && main_definite && !line.is_empty() && used + addition > main + 0.01 {
                lines.push(std::mem::take(&mut line));
                used = 0.0;
            }
            used += outer(item) + if line.is_empty() { 0.0 } else { gap };
            line.push(i);
        }
        if !line.is_empty() {
            lines.push(line);
        }
        let mut cross_sizes = Vec::new();
        for line in &lines {
            let fixed = gap * line.len().saturating_sub(1) as f32
                + line
                    .iter()
                    .map(|&i| {
                        if column {
                            items[i].margin.vertical() + items[i].inset.vertical()
                        } else {
                            items[i].margin.horizontal() + items[i].inset.horizontal()
                        }
                    })
                    .sum::<f32>();
            let grow = main - fixed - line.iter().map(|&i| items[i].base).sum::<f32>() >= 0.0;
            let mut frozen = vec![false; line.len()];
            for &i in line {
                items[i].size = items[i].base;
            }
            for _ in 0..64 {
                let free = main - fixed - line.iter().map(|&i| items[i].size).sum::<f32>();
                let weight = line
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| !frozen[*j])
                    .map(|(_, &i)| {
                        if grow {
                            items[i].grow
                        } else {
                            items[i].shrink * items[i].base
                        }
                    })
                    .sum::<f32>();
                if weight <= 0.0 || free.abs() < 0.01 {
                    break;
                }
                let mut clamped = false;
                for (j, &i) in line.iter().enumerate() {
                    if frozen[j] {
                        continue;
                    }
                    let weight = if grow {
                        items[i].grow
                    } else {
                        items[i].shrink * items[i].base
                    };
                    let target = items[i].size + free * weight / weight.max(0.000001).max(0.0);
                    let _ = target;
                    // All unfrozen items use the same denominator from this round.
                }
                let total_weight = weight;
                let free = if grow && weight < 1.0 {
                    free * weight
                } else {
                    free
                };
                for (j, &i) in line.iter().enumerate() {
                    if frozen[j] {
                        continue;
                    }
                    let w = if grow {
                        items[i].grow
                    } else {
                        items[i].shrink * items[i].base
                    };
                    let target = items[i].size + free * w / total_weight;
                    let size = target.clamp(items[i].min, items[i].max);
                    items[i].size = size;
                    if (size - target).abs() > 0.01 {
                        frozen[j] = true;
                        clamped = true;
                    }
                }
                if !clamped {
                    break;
                }
            }
            let mut cross = 0.0f32;
            let mut baseline = 0.0f32;
            for &i in line {
                items[i].size = items[i].size.clamp(items[i].min, items[i].max);
                let force = if column {
                    UsedSize {
                        height: Some(items[i].size),
                        width: None,
                    }
                } else {
                    UsedSize {
                        width: Some(items[i].size),
                        height: None,
                    }
                };
                let (size, base) = self.measure(items[i].node, content.width, height, force);
                items[i].cross = if column {
                    size.width + items[i].margin.horizontal()
                } else {
                    size.height + items[i].margin.vertical()
                };
                items[i].baseline = base + items[i].margin.top;
                cross = cross.max(items[i].cross);
                baseline = baseline.max(items[i].baseline);
            }
            if !column && style.align_items == Align::Baseline {
                for &i in line {
                    cross = cross.max(baseline + items[i].cross - items[i].baseline);
                }
            }
            cross_sizes.push(cross);
        }
        let natural =
            cross_sizes.iter().sum::<f32>() + cross_gap * lines.len().saturating_sub(1) as f32;
        let cross_limit = if column {
            content.width
        } else {
            height.unwrap_or(natural)
        };
        let extra = (cross_limit - natural).max(0.0);
        if matches!(style.align_content, Align::Normal | Align::Stretch) && !lines.is_empty() {
            for size in &mut cross_sizes {
                *size += extra / lines.len() as f32;
            }
        }
        if !style.wrap && cross_sizes.len() == 1 {
            cross_sizes[0] = cross_limit;
        }
        let (mut cross_cursor, cross_extra) = distribute(
            style.align_content,
            if style.wrap { extra } else { 0.0 },
            lines.len(),
        );
        for (line_no, line) in lines.iter().enumerate() {
            let line_cross = cross_sizes[line_no];
            let used = line.iter().map(|&i| outer(&items[i])).sum::<f32>()
                + gap * line.len().saturating_sub(1) as f32;
            let auto_count = line
                .iter()
                .map(|&i| {
                    let m = &self.tree.nodes[items[i].node].style.margin;
                    if column {
                        usize::from(matches!(m[0], SizeValue::Auto))
                            + usize::from(matches!(m[2], SizeValue::Auto))
                    } else {
                        usize::from(matches!(m[3], SizeValue::Auto))
                            + usize::from(matches!(m[1], SizeValue::Auto))
                    }
                })
                .sum::<usize>();
            let auto = if auto_count > 0 {
                (main - used).max(0.0) / auto_count as f32
            } else {
                0.0
            };
            let (mut main_cursor, space) = if auto_count == 0 {
                distribute(style.justify_content, main - used, line.len())
            } else {
                (0.0, 0.0)
            };
            let baseline = line.iter().map(|&i| items[i].baseline).fold(0.0, f32::max);
            for &i in line {
                let it = &items[i];
                let n = &self.tree.nodes[it.node];
                let align = if n.style.layout.align_self == Align::Auto {
                    style.align_items
                } else {
                    n.style.layout.align_self
                };
                let start_auto =
                    matches!(n.style.margin[if column { 0 } else { 3 }], SizeValue::Auto);
                let end_auto =
                    matches!(n.style.margin[if column { 2 } else { 1 }], SizeValue::Auto);
                main_cursor += if start_auto { auto } else { 0.0 };
                let axis_border = it.size
                    + if column {
                        it.inset.vertical()
                    } else {
                        it.inset.horizontal()
                    };
                let mut force = if column {
                    UsedSize {
                        height: Some(it.size),
                        width: None,
                    }
                } else {
                    UsedSize {
                        width: Some(it.size),
                        height: None,
                    }
                };
                let cross_dimension = if column {
                    &n.style.width
                } else {
                    &n.style.height
                };
                let cross_start_auto =
                    matches!(n.style.margin[if column { 3 } else { 0 }], SizeValue::Auto);
                let cross_end_auto =
                    matches!(n.style.margin[if column { 1 } else { 2 }], SizeValue::Auto);
                if matches!(align, Align::Normal | Align::Stretch)
                    && matches!(cross_dimension, SizeValue::Auto)
                    && !cross_start_auto
                    && !cross_end_auto
                {
                    if column {
                        force.width = Some(
                            (line_cross - it.margin.horizontal() - it.inset.horizontal()).max(0.0),
                        );
                    } else {
                        force.height = Some(
                            (line_cross - it.margin.vertical() - it.inset.vertical()).max(0.0),
                        );
                    }
                }
                let root = self.item(it.node, content, content.y, height, ancestors, force);
                let rect = self.result.boxes[root].border_box;
                let cross_border = if column {
                    rect.width + it.margin.horizontal()
                } else {
                    rect.height + it.margin.vertical()
                };
                let cross_offset = if cross_start_auto || cross_end_auto {
                    if cross_start_auto {
                        (line_cross - cross_border).max(0.0)
                            / (usize::from(cross_start_auto) + usize::from(cross_end_auto)) as f32
                    } else {
                        0.0
                    }
                } else {
                    match align {
                        Align::End => (line_cross - cross_border).max(0.0),
                        Align::Center => (line_cross - cross_border) / 2.0,
                        Align::Baseline if !column => baseline - it.baseline,
                        _ => 0.0,
                    }
                };
                let rtl = !column && node.style.get_property_value("direction") == "rtl";
                let reverse = style.reverse ^ rtl;
                let main_pos = if reverse {
                    main - main_cursor
                        - axis_border
                        - if column {
                            it.margin.bottom
                        } else {
                            it.margin.right
                        }
                } else {
                    main_cursor
                        + if column {
                            it.margin.top
                        } else {
                            it.margin.left
                        }
                };
                let cross_pos = if style.wrap_reverse {
                    cross_limit - cross_cursor - line_cross + cross_offset
                } else {
                    cross_cursor + cross_offset
                };
                let (x, y) = if column {
                    (content.x + cross_pos + it.margin.left, content.y + main_pos)
                } else {
                    (content.x + main_pos, content.y + cross_pos + it.margin.top)
                };
                self.shift_box(root, x - rect.x, y - rect.y);
                self.result.boxes[parent].content.push(Content::Box(root));
                main_cursor += axis_border
                    + if column {
                        it.margin.vertical()
                    } else {
                        it.margin.horizontal()
                    }
                    + if end_auto { auto } else { 0.0 }
                    + gap
                    + space;
            }
            cross_cursor += line_cross + cross_gap + cross_extra;
        }
        if column {
            main
        } else {
            cross_limit
        }
    }
}
