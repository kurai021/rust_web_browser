//! Canvas2D host surface; immutable frames go through the existing image painter.
use super::*;
use crate::canvas::{font_style, Canvas};
use dom_bindings::{css, html::NodeId};
#[derive(Debug, Clone)]
pub struct CanvasImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
    pub tainted: bool,
}
fn dimensions(handle: &DomHandle, node: NodeId) -> (u32, u32, u64) {
    handle.with_document(|d| {
        let dim = |key, default| {
            d.get_attribute(node, key)
                .and_then(|s| s.trim().parse::<u32>().ok())
                .unwrap_or(default)
        };
        (dim("width", 300), dim("height", 150), d.canvas_epoch(node))
    })
}
fn color_text(c: [u8; 4]) -> String {
    if c[3] == 255 {
        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
    } else {
        format!(
            "rgba({}, {}, {}, {})",
            c[0],
            c[1],
            c[2],
            c[3] as f64 / 255.0
        )
    }
}
fn color(text: &str) -> Option<[u8; 4]> {
    if text.len() > 512 {
        return None;
    }
    let declaration = css::syntax::parse_declarations(&format!("color:{text}"));
    let css::values::ColorValue::Rgba(c) = css::values::parse_color(&declaration.first()?.value)?
    else {
        return None;
    };
    Some([c.r, c.g, c.b, c.a])
}
impl WebHost {
    fn canvas_node(&self, id: u64) -> Option<NodeId> {
        let s = self.state.borrow();
        let node = s.dom.node_id(id)?;
        s.dom
            .with_document(|d| d.is_element_named(node, "canvas"))
            .then_some(node)
    }
    fn context_node(&self, id: u64) -> Option<NodeId> {
        match self.state.borrow().objects.get(&id).map(|o| &o.kind) {
            Some(Kind::CanvasContext { node }) => Some(*node),
            _ => None,
        }
    }
    fn ensure_canvas(&self, vm: &mut Vm, node: NodeId) -> std::result::Result<(), JsError> {
        let handle = self.state.borrow().dom.clone();
        let (width, height, epoch) = dimensions(&handle, node);
        if self
            .state
            .borrow()
            .canvases
            .get(&node)
            .is_some_and(|c| c.width == width && c.height == height && c.epoch == epoch)
        {
            return Ok(());
        }
        let mut canvas = Canvas::new(width, height).map_err(|e| error(vm, e))?;
        canvas.epoch = epoch;
        let mut s = self.state.borrow_mut();
        if !s.canvases.contains_key(&node) && s.canvases.len() >= 64 {
            return Err(error(
                vm,
                ApiError::new("QuotaExceededError", "canvas count"),
            ));
        }
        let used = s
            .canvases
            .iter()
            .filter(|(n, _)| **n != node)
            .map(|(_, c)| c.pixels.len())
            .sum::<usize>();
        if used.saturating_add(canvas.pixels.len()) > 32 * 1024 * 1024 {
            return Err(error(
                vm,
                ApiError::new("QuotaExceededError", "page canvas quota"),
            ));
        }
        s.canvases.insert(node, canvas);
        Ok(())
    }
    pub(super) fn canvas_get(
        &self,
        vm: &mut Vm,
        id: u64,
        key: &str,
    ) -> std::result::Result<Option<JsValue>, JsError> {
        if let Some(node) = self.canvas_node(id) {
            return Ok(match key {
                "getContext" => Some(self.state.borrow().function("canvas:getContext")),
                "width" | "height" => {
                    let (w, h, _) = dimensions(&self.state.borrow().dom, node);
                    Some(JsValue::Number(if key == "width" { w } else { h } as f64))
                }
                _ => None,
            });
        }
        if let Some(node) = self.context_node(id) {
            self.ensure_canvas(vm, node)?;
            let s = self.state.borrow();
            let c = &s.canvases[&node];
            return Ok(match key {
                "canvas" => Some(wrap(vm, s.dom.node_host(node), "HTMLCanvasElement")?),
                "fillStyle" => Some(JsValue::string(color_text(c.fill))),
                "strokeStyle" => Some(JsValue::string(color_text(c.stroke))),
                "lineWidth" => Some(JsValue::Number(c.line_width)),
                "globalAlpha" => Some(JsValue::Number(c.alpha)),
                "font" => Some(JsValue::string(&c.font)),
                "textAlign" => Some(JsValue::string(&c.text_align)),
                "textBaseline" => Some(JsValue::string(&c.text_baseline)),
                "fillRect" | "clearRect" | "strokeRect" | "beginPath" | "closePath" | "moveTo"
                | "lineTo" | "rect" | "arc" | "quadraticCurveTo" | "bezierCurveTo" | "fill"
                | "stroke" | "save" | "restore" | "translate" | "scale" | "rotate"
                | "transform" | "setTransform" | "resetTransform" | "drawImage" | "fillText"
                | "measureText" | "getImageData" | "putImageData" => {
                    Some(s.function(&format!("canvas:{key}")))
                }
                _ => None,
            });
        }
        let s = self.state.borrow();
        if let Some(node) = s
            .dom
            .node_id(id)
            .filter(|&n| s.dom.with_document(|d| d.is_element_named(n, "img")))
        {
            let current = s.dom.with_document(|d| d.image_epoch(node));
            let image = s
                .images
                .get(&node)
                .filter(|_| s.image_versions.get(&node) == Some(&current));
            return Ok(match key {
                "naturalWidth" => Some(JsValue::Number(image.map_or(0, |i| i.width) as f64)),
                "naturalHeight" => Some(JsValue::Number(image.map_or(0, |i| i.height) as f64)),
                "complete" => Some(JsValue::Bool(
                    s.image_versions.get(&node) == Some(&current)
                        || s.dom.with_document(|d| {
                            d.get_attribute(node, "src").is_none_or(str::is_empty)
                        }),
                )),
                _ => None,
            });
        }
        Ok(None)
    }
    pub(super) fn canvas_set(
        &self,
        vm: &mut Vm,
        id: u64,
        key: &str,
        value: JsValue,
    ) -> std::result::Result<bool, JsError> {
        if let Some(node) = self.canvas_node(id) {
            if matches!(key, "width" | "height") {
                let n = vm.to_number(value)?;
                let n = if n.is_finite() {
                    n.trunc().rem_euclid(4_294_967_296.0) as u32
                } else {
                    0
                };
                let handle = self.state.borrow().dom.clone();
                let (w, h, _) = dimensions(&handle, node);
                Canvas::new(
                    if key == "width" { n } else { w },
                    if key == "height" { n } else { h },
                )
                .map_err(|e| error(vm, e))?;
                handle
                    .update_document(|d| d.set_attribute(node, key, &n.to_string()))
                    .map_err(|e| error(vm, ApiError::new("InvalidStateError", e.to_string())))?;
                self.ensure_canvas(vm, node)?;
                return Ok(true);
            }
            return Ok(false);
        }
        let Some(node) = self.context_node(id) else {
            return Ok(false);
        };
        self.ensure_canvas(vm, node)?;
        let string = if matches!(
            key,
            "fillStyle" | "strokeStyle" | "font" | "textAlign" | "textBaseline"
        ) {
            Some(vm.to_string(value.clone())?)
        } else {
            None
        };
        let number = if matches!(key, "lineWidth" | "globalAlpha") {
            Some(vm.to_number(value)?)
        } else {
            None
        };
        let parsed_color = if matches!(key, "fillStyle" | "strokeStyle") {
            string.as_deref().and_then(color)
        } else {
            None
        };
        let font = if key == "font" {
            string.as_deref().and_then(font_style)
        } else {
            None
        };
        let mut s = self.state.borrow_mut();
        let c = s
            .canvases
            .get_mut(&node)
            .ok_or_else(|| error(vm, ApiError::new("InvalidStateError", "canvas")))?;
        match key {
            "fillStyle" => {
                if let Some(color) = parsed_color {
                    c.fill = color;
                }
            }
            "strokeStyle" => {
                if let Some(color) = parsed_color {
                    c.stroke = color;
                }
            }
            "lineWidth" => {
                if let Some(n) = number.filter(|n| n.is_finite() && *n > 0.0) {
                    c.line_width = n;
                }
            }
            "globalAlpha" => {
                if let Some(n) = number.filter(|n| (0.0..=1.0).contains(n)) {
                    c.alpha = n;
                }
            }
            "font" => {
                if let Some(font) = font {
                    if font.font_size > 256.0 {
                        return Err(error(
                            vm,
                            ApiError::new("QuotaExceededError", "canvas font size"),
                        ));
                    }
                    c.font = string.unwrap_or_default();
                    c.font_style = Arc::new(font);
                }
            }
            "textAlign" => {
                let v = string.unwrap_or_default();
                if matches!(v.as_str(), "left" | "right" | "center" | "start" | "end") {
                    c.text_align = v;
                }
            }
            "textBaseline" => {
                let v = string.unwrap_or_default();
                if matches!(
                    v.as_str(),
                    "top" | "hanging" | "middle" | "alphabetic" | "ideographic" | "bottom"
                ) {
                    c.text_baseline = v;
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    pub(super) fn canvas_call(
        &self,
        vm: &mut Vm,
        name: &str,
        id: u64,
        args: &[JsValue],
    ) -> std::result::Result<Option<JsValue>, JsError> {
        if !name.starts_with("canvas:") {
            return Ok(None);
        }
        vm.checkpoint()?;
        if name == "canvas:getContext" {
            let kind = text(vm, args, 0)?;
            let node = self
                .canvas_node(id)
                .ok_or_else(|| error(vm, ApiError::new("TypeError", "canvas receiver")))?;
            if kind != "2d" {
                return Ok(Some(JsValue::Null));
            }
            self.ensure_canvas(vm, node)?;
            let existing = self.state.borrow().canvas_contexts.get(&node).copied();
            let id = if let Some(id) = existing {
                id
            } else {
                let mut s = self.state.borrow_mut();
                let id = s
                    .allocate(Kind::CanvasContext { node })
                    .map_err(|e| error(vm, e))?;
                s.canvas_contexts.insert(node, id);
                id
            };
            return wrap(vm, id, "CanvasRenderingContext2D").map(Some);
        }
        let node = self
            .context_node(id)
            .ok_or_else(|| error(vm, ApiError::new("TypeError", "2D context receiver")))?;
        self.ensure_canvas(vm, node)?;
        if name == "canvas:fillText" || name == "canvas:measureText" {
            return self.canvas_text(vm, node, name, args).map(Some);
        }
        if name == "canvas:drawImage" {
            let handle = self.state.borrow().dom.clone();
            let source = vm
                .host_id(&arg(args, 0))
                .and_then(|id| handle.node_id(id))
                .ok_or_else(|| error(vm, ApiError::new("TypeError", "image source")))?;
            if handle.with_document(|d| d.is_element_named(source, "canvas")) {
                self.ensure_canvas(vm, source)?;
            }
            let image = {
                let s = self.state.borrow();
                if let Some(c) = s.canvases.get(&source) {
                    Some(CanvasImage {
                        width: c.width,
                        height: c.height,
                        pixels: c.pixels.clone().into(),
                        tainted: c.tainted,
                    })
                } else {
                    s.images
                        .get(&source)
                        .filter(|_| {
                            s.image_versions.get(&source)
                                == Some(&handle.with_document(|d| d.image_epoch(source)))
                        })
                        .cloned()
                }
            }
            .ok_or_else(|| error(vm, ApiError::new("InvalidStateError", "image not loaded")))?;
            if !matches!(args.len(), 3 | 5 | 9) {
                return Err(error(
                    vm,
                    ApiError::new("TypeError", "drawImage argument count"),
                ));
            }
            let mut n = Vec::new();
            for v in &args[1..] {
                n.push(vm.to_number(v.clone())?);
            }
            if n.iter().any(|n| !n.is_finite()) {
                return Ok(Some(JsValue::Undefined));
            }
            let (mut sx, mut sy, mut sw, mut sh, mut dx, mut dy, mut dw, mut dh) =
                match n.as_slice() {
                    [x, y] => (
                        0.0,
                        0.0,
                        image.width as f64,
                        image.height as f64,
                        *x,
                        *y,
                        image.width as f64,
                        image.height as f64,
                    ),
                    [x, y, w, h] => (
                        0.0,
                        0.0,
                        image.width as f64,
                        image.height as f64,
                        *x,
                        *y,
                        *w,
                        *h,
                    ),
                    [sx, sy, sw, sh, x, y, w, h] => (*sx, *sy, *sw, *sh, *x, *y, *w, *h),
                    _ => {
                        return Err(error(
                            vm,
                            ApiError::new("TypeError", "drawImage argument count"),
                        ))
                    }
                };
            for (pos, size) in [
                (&mut sx, &mut sw),
                (&mut sy, &mut sh),
                (&mut dx, &mut dw),
                (&mut dy, &mut dh),
            ] {
                if *size < 0.0 {
                    *pos += *size;
                    *size = -*size;
                }
            }
            let mut s = self.state.borrow_mut();
            let c = s
                .canvases
                .get_mut(&node)
                .ok_or_else(|| error(vm, ApiError::new("InvalidStateError", "canvas")))?;
            c.tainted |= image.tainted;
            c.transformed_image(
                &image.pixels,
                image.width,
                image.height,
                sx,
                sy,
                sw,
                sh,
                dx,
                dy,
                dw,
                dh,
            );
            return Ok(Some(JsValue::Undefined));
        }
        if name == "canvas:putImageData" {
            return self.put_image_data(vm, node, args).map(Some);
        }
        let count = match name {
            "canvas:moveTo" | "canvas:lineTo" | "canvas:translate" | "canvas:scale" => 2,
            "canvas:fillRect"
            | "canvas:clearRect"
            | "canvas:strokeRect"
            | "canvas:rect"
            | "canvas:quadraticCurveTo"
            | "canvas:getImageData" => 4,
            "canvas:arc" => 5,
            "canvas:transform" | "canvas:bezierCurveTo" => 6,
            "canvas:setTransform" => {
                if args.is_empty() {
                    0
                } else {
                    6
                }
            }
            "canvas:rotate" => 1,
            _ => 0,
        };
        let mut n = Vec::new();
        for i in 0..count {
            n.push(vm.to_number(arg(args, i))?);
        }
        if n.iter().any(|n| !n.is_finite()) {
            return Ok(Some(JsValue::Undefined));
        }
        let even_odd = if name == "canvas:fill" && arg(args, 0) != JsValue::Undefined {
            let value = text(vm, args, 0)?;
            if !matches!(value.as_str(), "nonzero" | "evenodd") {
                return Err(error(vm, ApiError::new("TypeError", "fill rule")));
            }
            value == "evenodd"
        } else {
            false
        };
        if name == "canvas:getImageData" {
            return self.get_image_data(vm, node, &n).map(Some);
        }
        let mut s = self.state.borrow_mut();
        let c = s
            .canvases
            .get_mut(&node)
            .ok_or_else(|| error(vm, ApiError::new("InvalidStateError", "canvas")))?;
        let result = match name {
            "canvas:beginPath" => {
                c.path.clear();
                Ok(())
            }
            "canvas:closePath" => {
                if let Some(path) = c.path.last_mut() {
                    if let Some(first) = path.first().copied() {
                        path.push(first);
                    }
                }
                Ok(())
            }
            "canvas:moveTo" => {
                let p = c.point(n[0], n[1]);
                c.path.push(vec![p]);
                Ok(())
            }
            "canvas:lineTo" => {
                let p = c.point(n[0], n[1]);
                if let Some(path) = c.path.last_mut() {
                    path.push(p);
                } else {
                    c.path.push(vec![p]);
                }
                Ok(())
            }
            "canvas:rect" | "canvas:fillRect" | "canvas:clearRect" | "canvas:strokeRect" => {
                let points = vec![
                    c.point(n[0], n[1]),
                    c.point(n[0] + n[2], n[1]),
                    c.point(n[0] + n[2], n[1] + n[3]),
                    c.point(n[0], n[1] + n[3]),
                    c.point(n[0], n[1]),
                ];
                if name == "canvas:rect" {
                    c.path.push(points);
                    Ok(())
                } else {
                    let path = std::mem::replace(&mut c.path, vec![points]);
                    let result = if name == "canvas:strokeRect" {
                        c.stroke_path()
                    } else if c.transform == [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] {
                        c.rect(n[0], n[1], n[2], n[3], name == "canvas:clearRect");
                        Ok(())
                    } else {
                        c.fill_path(name == "canvas:clearRect", false);
                        Ok(())
                    };
                    c.path = path;
                    result
                }
            }
            "canvas:arc" => {
                if n[2] < 0.0 {
                    Err(ApiError::new("IndexSizeError", "negative arc radius"))
                } else {
                    let tau = std::f64::consts::TAU;
                    let ccw = arg(args, 5).truthy();
                    let delta = n[4] - n[3];
                    let delta = if ccw {
                        if -delta >= tau {
                            -tau
                        } else {
                            -(-delta).rem_euclid(tau)
                        }
                    } else if delta >= tau {
                        tau
                    } else {
                        delta.rem_euclid(tau)
                    };
                    let steps = (delta.abs() * n[2].abs()).ceil().clamp(1.0, 256.0) as usize;
                    for i in 0..=steps {
                        let a = n[3] + delta * i as f64 / steps as f64;
                        let p = c.point(n[0] + n[2] * a.cos(), n[1] + n[2] * a.sin());
                        if c.path.is_empty() {
                            c.path.push(Vec::new());
                        }
                        if let Some(path) = c.path.last_mut() {
                            path.push(p);
                        }
                    }
                    Ok(())
                }
            }
            "canvas:quadraticCurveTo" | "canvas:bezierCurveTo" => {
                let a = c
                    .path
                    .last()
                    .and_then(|p| p.last())
                    .copied()
                    .unwrap_or_else(|| c.point(n[0], n[1]));
                let b = c.point(n[0], n[1]);
                let d = c.point(n[2], n[3]);
                let e = if n.len() == 6 { c.point(n[4], n[5]) } else { d };
                if c.path.is_empty() {
                    c.path.push(vec![a]);
                }
                for i in 1..=32 {
                    let t = i as f64 / 32.0;
                    let u = 1.0 - t;
                    let p = if n.len() == 6 {
                        (
                            u * u * u * a.0
                                + 3.0 * u * u * t * b.0
                                + 3.0 * u * t * t * d.0
                                + t * t * t * e.0,
                            u * u * u * a.1
                                + 3.0 * u * u * t * b.1
                                + 3.0 * u * t * t * d.1
                                + t * t * t * e.1,
                        )
                    } else {
                        (
                            u * u * a.0 + 2.0 * u * t * b.0 + t * t * d.0,
                            u * u * a.1 + 2.0 * u * t * b.1 + t * t * d.1,
                        )
                    };
                    if let Some(path) = c.path.last_mut() {
                        path.push(p);
                    }
                }
                Ok(())
            }
            "canvas:fill" => {
                c.fill_path(false, even_odd);
                Ok(())
            }
            "canvas:stroke" => c.stroke_path(),
            "canvas:save" => c.save(),
            "canvas:restore" => {
                c.restore();
                Ok(())
            }
            "canvas:translate" => {
                c.multiply([1.0, 0.0, 0.0, 1.0, n[0], n[1]]);
                Ok(())
            }
            "canvas:scale" => {
                c.multiply([n[0], 0.0, 0.0, n[1], 0.0, 0.0]);
                Ok(())
            }
            "canvas:rotate" => {
                c.multiply([n[0].cos(), n[0].sin(), -n[0].sin(), n[0].cos(), 0.0, 0.0]);
                Ok(())
            }
            "canvas:transform" => {
                c.multiply([n[0], n[1], n[2], n[3], n[4], n[5]]);
                Ok(())
            }
            "canvas:resetTransform" | "canvas:setTransform" => {
                c.transform = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
                if !n.is_empty() {
                    c.multiply([n[0], n[1], n[2], n[3], n[4], n[5]]);
                }
                Ok(())
            }
            _ => Err(ApiError::new("NotSupportedError", "canvas method")),
        };
        let path_points = c.path.iter().map(Vec::len).sum::<usize>();
        if path_points > 1024 {
            c.path.clear();
            return Err(error(
                vm,
                ApiError::new("QuotaExceededError", "canvas path points"),
            ));
        }
        result.map_err(|e| error(vm, e))?;
        drop(s);
        vm.checkpoint()?;
        Ok(Some(JsValue::Undefined))
    }
    fn get_image_data(
        &self,
        vm: &mut Vm,
        node: NodeId,
        n: &[f64],
    ) -> std::result::Result<JsValue, JsError> {
        let (mut x, mut y, mut w, mut h) = (n[0].trunc(), n[1].trunc(), n[2].trunc(), n[3].trunc());
        if w < 0.0 {
            x += w;
            w = -w;
        }
        if h < 0.0 {
            y += h;
            h = -h;
        }
        if w == 0.0 || h == 0.0 {
            return Err(error(
                vm,
                ApiError::new("IndexSizeError", "zero image data dimensions"),
            ));
        }
        if w * h * 4.0 > 262_144.0 {
            return Err(error(
                vm,
                ApiError::new("QuotaExceededError", "image data quota"),
            ));
        }
        let data = {
            let s = self.state.borrow();
            let c = &s.canvases[&node];
            if c.tainted {
                return Err(error(
                    vm,
                    ApiError::new("SecurityError", "canvas is not origin-clean"),
                ));
            }
            let mut data = vec![0u8; w as usize * h as usize * 4];
            for yy in 0..h as usize {
                for xx in 0..w as usize {
                    let (px, py) = (x + xx as f64, y + yy as f64);
                    if px >= 0.0 && py >= 0.0 && px < c.width as f64 && py < c.height as f64 {
                        let from = (py as usize * c.width as usize + px as usize) * 4;
                        let to = (yy * w as usize + xx) * 4;
                        data[to..to + 4].copy_from_slice(&c.pixels[from..from + 4]);
                    }
                }
            }
            data
        };
        let value = vm.object()?;
        let data = vm.array(
            data.into_iter()
                .map(|b| JsValue::Number(b as f64))
                .collect(),
        )?;
        vm.set(value.clone(), "data", data)?;
        vm.set(value.clone(), "width", JsValue::Number(w))?;
        vm.set(value.clone(), "height", JsValue::Number(h))?;
        Ok(value)
    }
    fn put_image_data(
        &self,
        vm: &mut Vm,
        node: NodeId,
        args: &[JsValue],
    ) -> std::result::Result<JsValue, JsError> {
        let image = arg(args, 0);
        let w = get(vm, image.clone(), "width")?;
        let h = get(vm, image.clone(), "height")?;
        let (w, h) = (vm.to_number(w)? as usize, vm.to_number(h)? as usize);
        let data = get(vm, image, "data")?;
        let data = vm.array_values(data)?;
        if w.checked_mul(h).and_then(|n| n.checked_mul(4)) != Some(data.len())
            || data.len() > 262_144
        {
            return Err(error(
                vm,
                ApiError::new("QuotaExceededError", "image data dimensions"),
            ));
        }
        let dx = vm.to_number(arg(args, 1))?;
        let dy = vm.to_number(arg(args, 2))?;
        if !dx.is_finite() || !dy.is_finite() {
            return Ok(JsValue::Undefined);
        }
        let mut bytes = Vec::new();
        for value in data {
            bytes.push(vm.to_number(value)?.round().clamp(0.0, 255.0) as u8);
        }
        let mut s = self.state.borrow_mut();
        let c = s
            .canvases
            .get_mut(&node)
            .ok_or_else(|| error(vm, ApiError::new("InvalidStateError", "canvas")))?;
        for y in 0..h {
            for x in 0..w {
                let (px, py) = (dx + x as f64, dy + y as f64);
                if px < 0.0 || py < 0.0 || px >= c.width as f64 || py >= c.height as f64 {
                    continue;
                }
                let to = (py as usize * c.width as usize + px as usize) * 4;
                let from = (y * w + x) * 4;
                c.pixels[to..to + 4].copy_from_slice(&bytes[from..from + 4]);
            }
        }
        c.dirty = true;
        Ok(JsValue::Undefined)
    }
    fn canvas_text(
        &self,
        vm: &mut Vm,
        node: NodeId,
        name: &str,
        args: &[JsValue],
    ) -> std::result::Result<JsValue, JsError> {
        use cosmic_text::{
            Attrs, Buffer, Family, Metrics, Shaping, Style, SwashCache, SwashContent, Weight, Wrap,
        };
        let text = text(vm, args, 0)?;
        if text.chars().count() > 512 {
            return Err(error(
                vm,
                ApiError::new("QuotaExceededError", "canvas text quota"),
            ));
        }
        let (x, y, max) = if name == "canvas:fillText" {
            (
                vm.to_number(arg(args, 1))?,
                vm.to_number(arg(args, 2))?,
                if args.len() > 3 {
                    Some(vm.to_number(arg(args, 3))?)
                } else {
                    None
                },
            )
        } else {
            (0.0, 0.0, None)
        };
        if !x.is_finite() || !y.is_finite() || max.is_some_and(|n| !n.is_finite() || n <= 0.0) {
            return Ok(JsValue::Undefined);
        }
        let mut s = self.state.borrow_mut();
        let font = s.canvases[&node].font_style.clone();
        let fonts = s
            .canvas_fonts
            .get_or_insert_with(cosmic_text::FontSystem::new);
        let mut buffer = Buffer::new(fonts, Metrics::new(font.font_size, font.font_size * 1.2));
        buffer.set_wrap(fonts, Wrap::None);
        buffer.set_size(fonts, None, None);
        let family = font
            .font_family
            .first()
            .map(String::as_str)
            .unwrap_or("sans-serif");
        let family = match family {
            "serif" => Family::Serif,
            "sans-serif" => Family::SansSerif,
            "monospace" => Family::Monospace,
            _ => Family::Name(family),
        };
        buffer.set_text(
            fonts,
            &text,
            Attrs::new()
                .family(family)
                .weight(Weight(font.font_weight))
                .style(if font.italic {
                    Style::Italic
                } else {
                    Style::Normal
                }),
            Shaping::Advanced,
        );
        buffer.shape_until_scroll(fonts, false);
        let width = buffer
            .layout_runs()
            .map(|r| r.line_w)
            .fold(0.0f32, f32::max);
        if name == "canvas:measureText" {
            drop(s);
            let value = vm.object()?;
            vm.set(value.clone(), "width", JsValue::Number(width as f64))?;
            vm.set(
                value.clone(),
                "actualBoundingBoxAscent",
                JsValue::Number(font.font_size as f64 * 0.8),
            )?;
            vm.set(
                value.clone(),
                "actualBoundingBoxDescent",
                JsValue::Number(font.font_size as f64 * 0.2),
            )?;
            return Ok(value);
        }
        let mut cache = SwashCache::new();
        let mut glyphs = Vec::new();
        let mut pixels = 0usize;
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                vm.checkpoint()?;
                let physical = glyph.physical((0.0, 0.0), 1.0);
                if let Some(image) = cache.get_image_uncached(fonts, physical.cache_key) {
                    pixels = pixels.saturating_add(
                        image.placement.width as usize * image.placement.height as usize,
                    );
                    if pixels > 4_000_000 {
                        return Err(error(
                            vm,
                            ApiError::new("QuotaExceededError", "canvas text raster quota"),
                        ));
                    }
                    glyphs.push((
                        physical.x + image.placement.left,
                        physical.y + run.line_y.round() as i32 - image.placement.top,
                        image,
                    ));
                }
            }
        }
        let c = s
            .canvases
            .get_mut(&node)
            .ok_or_else(|| error(vm, ApiError::new("InvalidStateError", "canvas")))?;
        let scale = max.map_or(1.0, |max| (max / width.max(0.01) as f64).min(1.0));
        let offset = match c.text_align.as_str() {
            "center" => width as f64 * scale / 2.0,
            "right" | "end" => width as f64 * scale,
            _ => 0.0,
        };
        let top = match c.text_baseline.as_str() {
            "top" | "hanging" => y,
            "middle" => y - font.font_size as f64 * 0.6,
            "bottom" | "ideographic" => y - font.font_size as f64 * 1.2,
            _ => y - font.font_size as f64 * 0.95,
        };
        for (gx, gy, image) in glyphs {
            let (w, h) = (image.placement.width, image.placement.height);
            let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
            match image.content {
                SwashContent::Color => rgba.extend_from_slice(&image.data),
                SwashContent::Mask => {
                    for a in image.data {
                        rgba.extend_from_slice(&[
                            c.fill[0],
                            c.fill[1],
                            c.fill[2],
                            ((a as u16 * c.fill[3] as u16) / 255) as u8,
                        ]);
                    }
                }
                SwashContent::SubpixelMask => {
                    for pixel in image.data.chunks(3) {
                        let a = pixel.iter().copied().max().unwrap_or(0);
                        rgba.extend_from_slice(&[
                            c.fill[0],
                            c.fill[1],
                            c.fill[2],
                            ((a as u16 * c.fill[3] as u16) / 255) as u8,
                        ]);
                    }
                }
            }
            c.transformed_image(
                &rgba,
                w,
                h,
                0.0,
                0.0,
                w as f64,
                h as f64,
                x - offset + gx as f64 * scale,
                top + gy as f64,
                w as f64 * scale,
                h as f64,
            );
        }
        drop(s);
        vm.checkpoint()?;
        Ok(JsValue::Undefined)
    }
}
impl PageRuntime {
    pub fn image_error(
        &mut self,
        node: NodeId,
        url: &Url,
        epoch: u64,
    ) -> std::result::Result<(), JsError> {
        self.dom.vm.begin_task();
        let handle = self.state.borrow().dom.clone();
        let valid = handle.with_document(|d| {
            d.is_element_named(node, "img")
                && d.image_epoch(node) == epoch
                && d.get_attribute(node, "src")
                    .and_then(|src| handle.url().join(src).ok())
                    .as_ref()
                    == Some(url)
        });
        if !valid || self.state.borrow().image_versions.get(&node) == Some(&epoch) {
            return Ok(());
        }
        let mut s = self.state.borrow_mut();
        s.images.remove(&node);
        s.image_versions.insert(node, epoch);
        drop(s);
        let result = self.dom.dispatch(node, "error", "").map(|_| ());
        self.finish_web_task(result)
    }
    pub(super) fn flush_canvases(&mut self) -> std::result::Result<(), JsError> {
        let host = WebHost {
            state: self.state.clone(),
        };
        let nodes = self
            .state
            .borrow()
            .canvases
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for node in nodes {
            host.ensure_canvas(&mut self.dom.vm, node)?;
        }
        let mut s = self.state.borrow_mut();
        let handle = s.dom.clone();
        for (&node, c) in &mut s.canvases {
            if c.dirty {
                handle
                    .update_document(|d| {
                        d.set_canvas_bitmap(node, c.width, c.height, Arc::from(c.pixels.as_slice()))
                    })
                    .map_err(|e| JsError::new(ErrorKind::MemoryLimit, e.to_string()))?;
                c.dirty = false;
            }
        }
        Ok(())
    }
    pub fn image(
        &mut self,
        node: NodeId,
        url: &Url,
        image: CanvasImage,
    ) -> std::result::Result<(), JsError> {
        let epoch = self
            .state
            .borrow()
            .dom
            .with_document(|d| d.image_epoch(node));
        self.image_at(node, url, epoch, image)
    }
    pub fn image_at(
        &mut self,
        node: NodeId,
        url: &Url,
        epoch: u64,
        image: CanvasImage,
    ) -> std::result::Result<(), JsError> {
        self.dom.vm.begin_task();
        let handle = self.state.borrow().dom.clone();
        let matches = handle.with_document(|d| {
            d.is_element_named(node, "img")
                && d.get_attribute(node, "src")
                    .and_then(|s| handle.url().join(s).ok())
                    .as_ref()
                    == Some(url)
        });
        if !matches {
            return Ok(());
        }
        if handle.with_document(|d| d.image_epoch(node)) != epoch {
            return Ok(());
        }
        if self.state.borrow().image_versions.get(&node) == Some(&epoch) {
            return Ok(());
        }
        if image.width > 4096
            || image.height > 4096
            || (image.width as usize)
                .checked_mul(image.height as usize)
                .and_then(|n| n.checked_mul(4))
                != Some(image.pixels.len())
        {
            return Err(JsError::new(ErrorKind::MemoryLimit, "canvas image bounds"));
        }
        {
            let mut s = self.state.borrow_mut();
            let used = s
                .images
                .iter()
                .filter(|(id, _)| **id != node)
                .map(|(_, i)| i.pixels.len())
                .sum::<usize>();
            if used.saturating_add(image.pixels.len()) > 64 * 1024 * 1024 {
                return Err(JsError::new(
                    ErrorKind::MemoryLimit,
                    "canvas source image quota",
                ));
            }
            s.images.insert(node, image);
            s.image_versions.insert(node, epoch);
        }
        self.dom.dispatch(node, "load", "")?;
        self.drain_jobs()
    }
}
