//! Origins, !important, layers, inline declarations, inheritance and var()
//! (plan/06 §6.2). Traverses the DOM in parent-first document order.

use crate::conditions::Environment;
use crate::selectors::{MatchContext, PseudoElement, Specificity};
use crate::style::{
    contains_var, declaration_keys, expand_property, inherited, initial_properties,
    supported_property, ComputedStyle, Properties,
};
use crate::stylesheet::{parse_stylesheet, FontFace, Rule, StyleRule, Stylesheet};
use crate::syntax::{block_end, parse_declarations, split_top_level, trim, Declaration};
use crate::token::Token;
use crate::values::{keyword, wide_keyword};
use html::{Document, NodeData, NodeId};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    UserAgent,
    User,
    Author,
}

#[derive(Debug, Clone)]
pub struct MatchedRule {
    pub origin: Origin,
    pub specificity: Specificity,
    pub source_order: usize,
    pub declarations: Vec<Declaration>,
    pub pseudo_element: Option<PseudoElement>,
}

#[derive(Debug, Clone, Default)]
pub struct ComputedStyles {
    elements: Vec<Option<Arc<ComputedStyle>>>,
    pseudo: BTreeMap<(NodeId, PseudoElement), Arc<ComputedStyle>>,
}
impl ComputedStyles {
    pub fn get(&self, node: NodeId) -> Option<&Arc<ComputedStyle>> {
        self.elements.get(node)?.as_ref()
    }
    pub fn pseudo(&self, node: NodeId, pseudo: PseudoElement) -> Option<&Arc<ComputedStyle>> {
        self.pseudo.get(&(node, pseudo))
    }
}

pub struct Cascade {
    pub ua: Stylesheet,
    pub user: Stylesheet,
    pub environment: Environment,
    pub context: MatchContext,
}
impl Default for Cascade {
    fn default() -> Self {
        Self {
            ua: parse_stylesheet(include_str!("ua.css"), None),
            user: Stylesheet::default(),
            environment: Environment::default(),
            context: MatchContext::default(),
        }
    }
}

impl Cascade {
    pub fn compute(&self, doc: &Document, author_sheets: &[Stylesheet]) -> ComputedStyles {
        if doc.node_count() == 0 {
            return ComputedStyles::default();
        }
        let flat = self.flatten(author_sheets);
        let mut styles = ComputedStyles {
            elements: vec![None; doc.node_count()],
            pseudo: BTreeMap::new(),
        };
        let mut stack = vec![(doc.root(), None)];
        let mut root_font_size = 16.0;
        while let Some((node, parent)) = stack.pop() {
            let mut next_parent = parent;
            if matches!(doc.get(node).data, NodeData::Element(_)) {
                let inherited_style = parent.and_then(|id| styles.get(id)).map(|s| &**s);
                let computed = self.compute_node(
                    doc,
                    node,
                    inherited_style,
                    &flat,
                    self.context,
                    root_font_size,
                    true,
                );
                if doc.document_element() == Some(node) {
                    root_font_size = computed.font_size;
                }
                styles.elements[node] = Some(Arc::new(computed));
                // Pseudo styles share the originating element's inherited
                // values, never its non-inherited box properties.
                for pseudo in [
                    PseudoElement::Before,
                    PseudoElement::After,
                    PseudoElement::FirstLine,
                    PseudoElement::FirstLetter,
                    PseudoElement::Selection,
                    PseudoElement::Marker,
                ] {
                    let ctx = MatchContext {
                        pseudo_element: Some(pseudo),
                        ..self.context
                    };
                    if flat.rules.iter().any(|r| {
                        r.rule
                            .selectors
                            .iter()
                            .any(|s| s.pseudo_element == Some(pseudo) && s.matches(doc, node, ctx))
                    }) {
                        let parent = styles.get(node).map(|s| &**s);
                        let style =
                            self.compute_node(doc, node, parent, &flat, ctx, root_font_size, false);
                        styles.pseudo.insert((node, pseudo), Arc::new(style));
                    }
                }
                next_parent = Some(node);
            }
            for &child in doc.get(node).children.iter().rev() {
                stack.push((child, next_parent));
            }
        }
        styles
    }

    pub fn matching_rules(
        &self,
        doc: &Document,
        node: NodeId,
        author_sheets: &[Stylesheet],
    ) -> Vec<MatchedRule> {
        self.flatten(author_sheets)
            .rules
            .into_iter()
            .filter_map(|r| {
                let specificity = r
                    .rule
                    .selectors
                    .iter()
                    .filter(|s| s.matches(doc, node, self.context))
                    .map(|s| s.specificity)
                    .max()?;
                Some(MatchedRule {
                    origin: r.origin,
                    specificity,
                    source_order: r.order,
                    declarations: r.rule.declarations.clone(),
                    pseudo_element: self.context.pseudo_element,
                })
            })
            .collect()
    }

    pub fn font_faces(&self, author_sheets: &[Stylesheet]) -> Vec<FontFace> {
        self.flatten(author_sheets).fonts
    }

    fn flatten<'a>(&'a self, author_sheets: &'a [Stylesheet]) -> Flat<'a> {
        let mut flat = Flat::default();
        for (origin, sheet) in [(Origin::UserAgent, &self.ua), (Origin::User, &self.user)] {
            flat.walk(
                &sheet.rules,
                sheet.base_url.as_ref(),
                origin,
                None,
                self.environment,
                0,
            );
        }
        for sheet in author_sheets {
            flat.walk(
                &sheet.rules,
                sheet.base_url.as_ref(),
                Origin::Author,
                None,
                self.environment,
                0,
            );
        }
        flat
    }

    #[allow(clippy::too_many_arguments)]
    fn compute_node(
        &self,
        doc: &Document,
        node: NodeId,
        parent: Option<&ComputedStyle>,
        flat: &Flat<'_>,
        ctx: MatchContext,
        root_font_size: f32,
        inline: bool,
    ) -> ComputedStyle {
        let mut candidates: BTreeMap<String, Vec<Candidate>> = BTreeMap::new();
        for rule in &flat.rules {
            let Some(specificity) = rule
                .rule
                .selectors
                .iter()
                .filter(|s| s.matches(doc, node, ctx))
                .map(|s| s.specificity)
                .max()
            else {
                continue;
            };
            for decl in &rule.rule.declarations {
                add_candidate(
                    &mut candidates,
                    decl,
                    rule.origin,
                    rule.layer,
                    false,
                    specificity,
                    rule.order,
                    rule.base,
                );
            }
        }
        if inline {
            if let Some(raw) = doc.get_attribute(node, "style") {
                for decl in parse_declarations(raw) {
                    add_candidate(
                        &mut candidates,
                        &decl,
                        Origin::Author,
                        None,
                        true,
                        Specificity::default(),
                        flat.rules.len() + 1,
                        doc.url.as_ref(),
                    );
                }
            }
        }
        for values in candidates.values_mut() {
            values.sort_by_key(|c| std::cmp::Reverse(c.rank));
        }
        let mut custom = parent.map_or_else(Properties::new, |p| p.custom_properties.clone());
        for (name, values) in &candidates {
            if !name.starts_with("--") {
                continue;
            }
            if let Some(selected) = select_candidate(values) {
                match keyword(&selected.decl.value).as_deref() {
                    Some("initial") => {
                        custom.remove(name);
                    }
                    Some("inherit" | "unset") => {}
                    _ => {
                        custom.insert(name.clone(), selected.decl.value.clone());
                    }
                }
            }
        }
        let mut resolver = Variables {
            raw: &custom,
            cache: BTreeMap::new(),
            visiting: BTreeSet::new(),
        };
        let custom_names: Vec<String> = custom.keys().cloned().collect();
        let mut computed_custom = Properties::new();
        for name in custom_names {
            if let Some(value) = resolver.property(&name) {
                computed_custom.insert(name, value);
            }
        }
        let mut properties = initial_properties();
        for (name, initial) in properties.clone() {
            let candidate = candidates.get(&name).and_then(|cs| select_candidate(cs));
            let resolved = candidate.and_then(|c| {
                let value = if contains_var(&c.decl.value) {
                    resolver.substitute(&c.decl.value)?
                } else {
                    c.decl.value.clone()
                };
                let mut expanded = expand_property(&c.decl.name, &value)?;
                let value = expanded
                    .iter_mut()
                    .find(|(key, _)| key == &name)
                    .map(|(_, v)| std::mem::take(v))?;
                // Resolve URLs against their source sheet, not the document.
                if name == "background-image" {
                    if let Some((raw, _)) = crate::stylesheet::url_value(&value) {
                        let url = crate::stylesheet::resource_url(c.base.as_ref(), &raw)?;
                        return Some(vec![Token::Url(url.to_string())]);
                    }
                }
                Some(value)
            });
            let inherit_value = || {
                parent
                    .and_then(|p| p.properties.get(&name))
                    .cloned()
                    .unwrap_or_else(|| initial.clone())
            };
            let value = match resolved.as_ref().and_then(|v| wide_keyword(v)).as_deref() {
                Some("inherit") => inherit_value(),
                Some("unset") if inherited(&name) => inherit_value(),
                Some("initial" | "unset") => initial,
                _ => resolved.unwrap_or_else(|| {
                    if inherited(&name) {
                        inherit_value()
                    } else {
                        initial.clone()
                    }
                }),
            };
            properties.insert(name, value);
        }
        ComputedStyle::from_properties(
            properties,
            parent,
            root_font_size,
            self.environment,
            doc.url.as_ref(),
            computed_custom,
        )
    }
}

#[derive(Default)]
struct Flat<'a> {
    rules: Vec<FlatRule<'a>>,
    fonts: Vec<FontFace>,
    layers: BTreeMap<(Origin, String), u32>,
    anonymous: u32,
}
struct FlatRule<'a> {
    rule: &'a StyleRule,
    base: Option<&'a Url>,
    origin: Origin,
    layer: Option<u32>,
    order: usize,
}
impl<'a> Flat<'a> {
    fn register_layer(&mut self, origin: Origin, name: &str) -> u32 {
        let next = self.layers.keys().filter(|(o, _)| *o == origin).count() as u32;
        *self.layers.entry((origin, name.to_owned())).or_insert(next)
    }
    #[allow(clippy::too_many_arguments)]
    fn walk(
        &mut self,
        rules: &'a [Rule],
        base: Option<&'a Url>,
        origin: Origin,
        layer: Option<&str>,
        env: Environment,
        depth: usize,
    ) {
        if depth > 64 {
            return;
        }
        for rule in rules {
            match rule {
                Rule::Style(rule) => {
                    let layer = layer.map(|name| self.register_layer(origin, name));
                    self.rules.push(FlatRule {
                        rule,
                        base,
                        origin,
                        layer,
                        order: self.rules.len(),
                    });
                }
                Rule::Media(query, children) if query.matches(env) => {
                    self.walk(children, base, origin, layer, env, depth + 1)
                }
                Rule::Supports(query, children) if query.matches(supported_property) => {
                    self.walk(children, base, origin, layer, env, depth + 1)
                }
                Rule::LayerOrder(names) => {
                    for name in names {
                        let name =
                            layer.map_or_else(|| name.clone(), |parent| format!("{parent}.{name}"));
                        self.register_layer(origin, &name);
                    }
                }
                Rule::Layer { name, rules } => {
                    let name = name.clone().unwrap_or_else(|| {
                        self.anonymous += 1;
                        format!("<anonymous-{}>", self.anonymous)
                    });
                    let name = layer.map_or(name.clone(), |parent| format!("{parent}.{name}"));
                    self.register_layer(origin, &name);
                    self.walk(rules, base, origin, Some(&name), env, depth + 1);
                }
                Rule::Import(import) if import.media.matches(env) => {
                    if let Some(sheet) = &import.sheet {
                        if let Some(name) = &import.layer {
                            let name = if name.is_empty() {
                                self.anonymous += 1;
                                format!("<anonymous-{}>", self.anonymous)
                            } else {
                                name.clone()
                            };
                            let name =
                                layer.map_or(name.clone(), |parent| format!("{parent}.{name}"));
                            self.register_layer(origin, &name);
                            self.walk(
                                &sheet.rules,
                                sheet.base_url.as_ref(),
                                origin,
                                Some(&name),
                                env,
                                depth + 1,
                            );
                        } else {
                            self.walk(
                                &sheet.rules,
                                sheet.base_url.as_ref(),
                                origin,
                                layer,
                                env,
                                depth + 1,
                            );
                        }
                    }
                }
                Rule::FontFace(face) => self.fonts.push(face.clone()),
                _ => {}
            }
        }
    }
}

type Rank = (u8, bool, u32, Specificity, usize, usize);
struct Candidate {
    decl: Declaration,
    origin: Origin,
    layer: Option<u32>,
    rank: Rank,
    base: Option<Url>,
}
#[allow(clippy::too_many_arguments)]
fn add_candidate(
    map: &mut BTreeMap<String, Vec<Candidate>>,
    decl: &Declaration,
    origin: Origin,
    layer: Option<u32>,
    inline: bool,
    specificity: Specificity,
    order: usize,
    base: Option<&Url>,
) {
    let level = match (decl.important, origin) {
        (false, Origin::UserAgent) => 0,
        (false, Origin::User) => 1,
        (false, Origin::Author) => 2,
        (true, Origin::Author) => 3,
        (true, Origin::User) => 4,
        (true, Origin::UserAgent) => 5,
    };
    let layer_rank = if decl.important {
        layer.map_or(0, |i| u32::MAX - i)
    } else {
        layer.unwrap_or(u32::MAX)
    };
    for key in declaration_keys(decl) {
        let values = map.entry(key).or_default();
        let serial = values.len();
        values.push(Candidate {
            decl: decl.clone(),
            origin,
            layer,
            rank: (level, inline, layer_rank, specificity, order, serial),
            base: base.cloned(),
        });
    }
}
fn select_candidate(candidates: &[Candidate]) -> Option<&Candidate> {
    let mut reverted = BTreeSet::new();
    let mut reverted_layers = BTreeSet::new();
    for candidate in candidates {
        if reverted.contains(&candidate.origin)
            || reverted_layers.contains(&(candidate.origin, candidate.layer))
        {
            continue;
        }
        match keyword(&candidate.decl.value).as_deref() {
            Some("revert") => {
                reverted.insert(candidate.origin);
            }
            Some("revert-layer") => {
                reverted_layers.insert((candidate.origin, candidate.layer));
            }
            _ => return Some(candidate),
        }
    }
    None
}

struct Variables<'a> {
    raw: &'a Properties,
    cache: BTreeMap<String, Option<Vec<Token>>>,
    visiting: BTreeSet<String>,
}
impl Variables<'_> {
    fn property(&mut self, name: &str) -> Option<Vec<Token>> {
        if let Some(value) = self.cache.get(name) {
            return value.clone();
        }
        if !self.visiting.insert(name.to_owned()) {
            self.cache.insert(name.into(), None);
            return None;
        }
        if self.visiting.len() > 32 {
            self.visiting.remove(name);
            return None;
        }
        let raw = self.raw.get(name).cloned();
        let value = raw.and_then(|v| self.substitute(&v));
        self.visiting.remove(name);
        // A cycle marker must not be overwritten by a fallback inside it.
        let value = if self.cache.get(name) == Some(&None) {
            None
        } else {
            value
        };
        self.cache.insert(name.into(), value.clone());
        value
    }
    fn substitute(&mut self, tokens: &[Token]) -> Option<Vec<Token>> {
        let mut output = Vec::new();
        let mut index = 0;
        while index < tokens.len() {
            if let Token::Function(name) = &tokens[index] {
                let (end, balanced) = block_end(tokens, index);
                if !balanced {
                    return None;
                }
                let args = &tokens[index + 1..end - 1];
                if name.eq_ignore_ascii_case("var") {
                    let parts = split_top_level(args, &Token::Comma);
                    let [Token::Ident(name)] = trim(parts[0]) else {
                        return None;
                    };
                    if !name.starts_with("--") {
                        return None;
                    }
                    let value = match self.property(name) {
                        Some(value) => value,
                        None if parts.len() > 1 => {
                            let mut fallback = Vec::new();
                            for (i, part) in parts[1..].iter().enumerate() {
                                if i > 0 {
                                    fallback.push(Token::Comma);
                                }
                                fallback.extend_from_slice(part);
                            }
                            self.substitute(&fallback)?
                        }
                        None => return None,
                    };
                    output.extend(value);
                } else {
                    output.push(tokens[index].clone());
                    output.extend(self.substitute(args)?);
                    output.push(Token::CloseParen);
                }
                index = end;
            } else {
                output.push(tokens[index].clone());
                index += 1;
            }
            if output.len() > 8192 {
                return None;
            }
        }
        Some(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::values::Color;
    fn fixture() -> Document {
        let url = "https://example.com/".parse().unwrap();
        html::parse_full(b"<!DOCTYPE html><div id='parent'><p id='target' class='a' style='color:purple; margin-left:7px'>Hi <b id='child'>there</b></p></div>",&url,html::ParseOpts::default())
    }
    #[test]
    fn origins_important_inline_and_inheritance() {
        let doc = fixture();
        let id = doc.get_element_by_id("target").unwrap();
        let child = doc.get_element_by_id("child").unwrap();
        let cascade = Cascade {
            user: parse_stylesheet("p{color:green !important}", None),
            ..Cascade::default()
        };
        let author=parse_stylesheet("#parent{font-size:20px;color:red} p.a{color:blue!important; margin:1px 2px} #target{font-size:150%}",None);
        let computed = cascade.compute(&doc, &[author]);
        let p = computed.get(id).unwrap();
        let b = computed.get(child).unwrap();
        assert_eq!(p.color, Color::rgb(0, 128, 0));
        assert_eq!(p.font_size, 30.0);
        assert_eq!(b.font_size, 30.0);
        assert_eq!(p.get_property_value("margin-left"), "7px");
        assert_eq!(p.get_property_value("margin-right"), "2px");
        assert_eq!(b.font_weight, 700);
    }
    #[test]
    fn variables_cycles_shorthands_and_media_resize() {
        let doc = fixture();
        let id = doc.get_element_by_id("target").unwrap();
        let author=parse_stylesheet("#parent{--gap:3px; --a:var(--b);--b:var(--a)} #target{color:var(--a,red)!important;padding:var(--gap);font-size:20px} @media(max-width:500px){#target{font-size:10px}}",None);
        let mut cascade = Cascade::default();
        let styles = cascade.compute(&doc, std::slice::from_ref(&author));
        let style = styles.get(id).unwrap();
        assert_eq!(style.color, Color::rgb(255, 0, 0));
        assert_eq!(style.get_property_value("padding-left"), "3px");
        assert_eq!(style.font_size, 20.0);
        cascade.environment.width = 400.0;
        assert_eq!(
            cascade.compute(&doc, &[author]).get(id).unwrap().font_size,
            10.0
        );
    }
    #[test]
    fn layer_order_reverses_for_important_and_revert_restores_origin() {
        let doc = fixture();
        let child = doc.get_element_by_id("child").unwrap();
        let author=parse_stylesheet("@layer first,last; @layer first{b{color:red!important}} @layer last{#child{color:blue!important}} b{color:green!important} b{font-size:40px;font-size:revert}",None);
        let styles = Cascade::default().compute(&doc, &[author]);
        let style = styles.get(child).unwrap();
        assert_eq!(style.color, Color::rgb(255, 0, 0));
        assert_eq!(style.font_size, 16.0);
    }
}
