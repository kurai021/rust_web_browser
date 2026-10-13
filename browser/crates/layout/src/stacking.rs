//! One retained stacking traversal shared by painting and hit testing.
use crate::*;
use css::layout_values::{Mode, Position};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintCommand {
    Start(usize),
    End(usize),
    Item(usize, usize),
    EffectsStart(usize),
    EffectsEnd(usize),
}
impl LayoutResult {
    fn context(&self, index: usize) -> bool {
        let b = &self.boxes[index];
        !b.anonymous
            && (b.parent.is_none()
                || b.style.opacity < 1.0
                || b.style.layout.position == Position::Fixed
                || b.style.layout.z_index.is_some()
                    && (b.style.layout.position != Position::Static
                        || b.parent.is_some_and(|p| {
                            matches!(self.boxes[p].style.layout.mode, Mode::Flex | Mode::Grid)
                        })))
    }
    fn positioned_layer(&self, index: usize) -> bool {
        !self.boxes[index].anonymous && self.boxes[index].style.layout.position != Position::Static
    }
    pub fn stacking_commands(&self) -> Vec<PaintCommand> {
        fn layers(
            result: &LayoutResult,
            root: usize,
            path: &mut Vec<usize>,
            out: &mut Vec<(i32, usize, Vec<usize>, bool)>,
        ) {
            for content in &result.boxes[root].content {
                if let Content::Box(child) = content {
                    if result.context(*child) {
                        out.push((
                            result.boxes[*child].style.layout.z_index.unwrap_or(0),
                            *child,
                            path.clone(),
                            true,
                        ));
                    } else {
                        if result.positioned_layer(*child) {
                            out.push((0, *child, path.clone(), false));
                        }
                        path.push(*child);
                        layers(result, *child, path, out);
                        path.pop();
                    }
                }
            }
        }
        fn normal(result: &LayoutResult, root: usize, out: &mut Vec<PaintCommand>) {
            for (i, item) in result.boxes[root].content.iter().enumerate() {
                match item {
                    Content::Box(child)
                        if !result.context(*child) && !result.positioned_layer(*child) =>
                    {
                        out.push(PaintCommand::Start(*child));
                        normal(result, *child, out);
                        out.push(PaintCommand::End(*child));
                    }
                    Content::Box(_) => {}
                    _ => out.push(PaintCommand::Item(root, i)),
                }
            }
        }
        fn context(result: &LayoutResult, root: usize, out: &mut Vec<PaintCommand>) {
            out.push(PaintCommand::Start(root));
            let mut children = Vec::new();
            layers(result, root, &mut Vec::new(), &mut children);
            // Stable DFS order resolves equal z-index values.
            children.sort_by_key(|(z, _, _, _)| *z);
            for negative in [true, false] {
                if !negative {
                    normal(result, root, out);
                }
                for (z, child, path, is_context) in &children {
                    if (*z < 0) != negative {
                        continue;
                    }
                    for &ancestor in path {
                        out.push(PaintCommand::EffectsStart(ancestor));
                    }
                    if *is_context {
                        context(result, *child, out);
                    } else {
                        out.push(PaintCommand::Start(*child));
                        normal(result, *child, out);
                        out.push(PaintCommand::End(*child));
                    }
                    for &ancestor in path.iter().rev() {
                        out.push(PaintCommand::EffectsEnd(ancestor));
                    }
                }
            }
            out.push(PaintCommand::End(root));
        }
        let mut out = Vec::new();
        for &root in &self.roots {
            context(self, root, &mut out);
        }
        out
    }
    pub(crate) fn finalize_paint(&mut self) {
        self.paint_order = self.stacking_commands();
        self.paint_rank = vec![0; self.boxes.len()];
        for (rank, cmd) in self.paint_order.iter().enumerate() {
            if let PaintCommand::Start(i) = cmd {
                self.paint_rank[*i] = rank;
            }
        }
    }
}
