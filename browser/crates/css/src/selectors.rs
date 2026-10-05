//! Selectors 4 subset, specificity and right-to-left DOM matching (plan/06).
//! Unsupported selectors invalidate their rule instead of accidentally
//! broadening it. Visited history is deliberately not exposed to styling.

use crate::syntax::{block_end, split_top_level, trim};
use crate::token::{serialize, tokenize_all, Token};
use html::{Document, Namespace, NodeData, NodeId};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Specificity(pub u32, pub u32, pub u32);

impl Specificity {
    fn add(self, other: Self) -> Self {
        Self(
            self.0.saturating_add(other.0),
            self.1.saturating_add(other.1),
            self.2.saturating_add(other.2),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PseudoElement {
    Before,
    After,
    FirstLine,
    FirstLetter,
    Selection,
    Marker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Combinator {
    Descendant,
    Child,
    Adjacent,
    Sibling,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Selector {
    compounds: Vec<Vec<Simple>>,
    combinators: Vec<Combinator>,
    pub specificity: Specificity,
    pub pseudo_element: Option<PseudoElement>,
}

#[derive(Debug, Clone, PartialEq)]
enum Simple {
    Universal,
    Type(String),
    Id(String),
    Class(String),
    Attribute(AttrSelector),
    Root,
    Empty,
    Link,
    Visited,
    Hover,
    Focus,
    Active,
    Checked,
    Disabled,
    Enabled,
    Nth {
        a: i64,
        b: i64,
        from_end: bool,
        of_type: bool,
        filter: Vec<Selector>,
    },
    Not(Vec<Selector>),
    Is(Vec<Selector>),
    Where(Vec<Selector>),
}

#[derive(Debug, Clone, PartialEq)]
struct AttrSelector {
    name: String,
    op: AttrOp,
    value: String,
    ignore_case: Option<bool>,
}
#[derive(Debug, Clone, Copy, PartialEq)]
enum AttrOp {
    Exists,
    Equals,
    Includes,
    Dash,
    Prefix,
    Suffix,
    Substring,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MatchContext {
    pub hovered: Option<NodeId>,
    pub focused: Option<NodeId>,
    pub active: Option<NodeId>,
    pub pseudo_element: Option<PseudoElement>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid or unsupported CSS selector")]
pub struct SelectorError;

pub fn parse_selector_list(css: &str) -> Result<Vec<Selector>, SelectorError> {
    parse_tokens(&tokenize_all(css))
}

pub(crate) fn parse_tokens(tokens: &[Token]) -> Result<Vec<Selector>, SelectorError> {
    parse_list_at(tokens, 0, false)
}

fn parse_list_at(
    tokens: &[Token],
    depth: usize,
    forgiving: bool,
) -> Result<Vec<Selector>, SelectorError> {
    if depth > 32 || tokens.len() > 8192 {
        return Err(SelectorError);
    }
    let mut selectors = Vec::new();
    for part in split_top_level(tokens, &Token::Comma) {
        match parse_one(trim(part), depth) {
            Ok(selector) => selectors.push(selector),
            Err(_) if forgiving => {}
            Err(err) => return Err(err),
        }
    }
    if selectors.is_empty() {
        Err(SelectorError)
    } else {
        Ok(selectors)
    }
}

fn parse_one(tokens: &[Token], depth: usize) -> Result<Selector, SelectorError> {
    let mut index = 0;
    let mut compounds = Vec::new();
    let mut combinators = Vec::new();
    let mut specificity = Specificity::default();
    let mut pseudo_element = None;
    while index < tokens.len() {
        if compounds.len() > 128 {
            return Err(SelectorError);
        }
        let mut compound = Vec::new();
        // Only the first simple selector of a compound can be a type.
        match tokens.get(index) {
            Some(Token::Ident(name)) => {
                compound.push(Simple::Type(name.clone()));
                specificity = specificity.add(Specificity(0, 0, 1));
                index += 1;
            }
            Some(Token::Delim('*')) => {
                compound.push(Simple::Universal);
                index += 1;
            }
            _ => {}
        }
        while index < tokens.len() {
            if pseudo_element.is_some() {
                break;
            }
            let simple = match &tokens[index] {
                Token::Hash { value, is_id: true } => {
                    index += 1;
                    Simple::Id(value.clone())
                }
                Token::Delim('.') => {
                    index += 1;
                    let Some(Token::Ident(name)) = tokens.get(index) else {
                        return Err(SelectorError);
                    };
                    index += 1;
                    Simple::Class(name.clone())
                }
                Token::OpenSquare => {
                    let (end, balanced) = block_end(tokens, index);
                    if !balanced {
                        return Err(SelectorError);
                    }
                    let attr = parse_attribute(&tokens[index + 1..end - 1])?;
                    index = end;
                    Simple::Attribute(attr)
                }
                Token::Colon => {
                    index += 1;
                    let double = tokens.get(index) == Some(&Token::Colon);
                    if double {
                        index += 1;
                    }
                    match tokens.get(index) {
                        Some(Token::Ident(name)) => {
                            let name = name.to_ascii_lowercase();
                            index += 1;
                            let pseudo = match name.as_str() {
                                "before" => Some(PseudoElement::Before),
                                "after" => Some(PseudoElement::After),
                                "first-line" => Some(PseudoElement::FirstLine),
                                "first-letter" => Some(PseudoElement::FirstLetter),
                                "selection" if double => Some(PseudoElement::Selection),
                                "marker" if double => Some(PseudoElement::Marker),
                                _ => None,
                            };
                            if let Some(pseudo) = pseudo {
                                if compound.is_empty() {
                                    compound.push(Simple::Universal);
                                }
                                pseudo_element = Some(pseudo);
                                specificity = specificity.add(Specificity(0, 0, 1));
                                continue;
                            }
                            if double {
                                return Err(SelectorError);
                            }
                            match name.as_str() {
                                "root" | "scope" => Simple::Root,
                                "empty" => Simple::Empty,
                                "link" | "any-link" => Simple::Link,
                                "visited" => Simple::Visited,
                                "hover" => Simple::Hover,
                                "focus" => Simple::Focus,
                                "active" => Simple::Active,
                                "checked" => Simple::Checked,
                                "disabled" => Simple::Disabled,
                                "enabled" => Simple::Enabled,
                                "first-child" => Simple::Nth {
                                    a: 0,
                                    b: 1,
                                    from_end: false,
                                    of_type: false,
                                    filter: Vec::new(),
                                },
                                "last-child" => Simple::Nth {
                                    a: 0,
                                    b: 1,
                                    from_end: true,
                                    of_type: false,
                                    filter: Vec::new(),
                                },
                                "only-child" => {
                                    Simple::Not(vec![nth_except(1, false), nth_except(1, true)])
                                }
                                "first-of-type" => Simple::Nth {
                                    a: 0,
                                    b: 1,
                                    from_end: false,
                                    of_type: true,
                                    filter: Vec::new(),
                                },
                                "last-of-type" => Simple::Nth {
                                    a: 0,
                                    b: 1,
                                    from_end: true,
                                    of_type: true,
                                    filter: Vec::new(),
                                },
                                _ => return Err(SelectorError),
                            }
                        }
                        Some(Token::Function(name)) if !double => {
                            let name = name.to_ascii_lowercase();
                            let (end, balanced) = block_end(tokens, index);
                            if !balanced {
                                return Err(SelectorError);
                            }
                            let args = &tokens[index + 1..end - 1];
                            index = end;
                            match name.as_str() {
                                "not" => Simple::Not(parse_list_at(args, depth + 1, false)?),
                                "is" => Simple::Is(parse_list_at(args, depth + 1, true)?),
                                "where" => Simple::Where(parse_list_at(args, depth + 1, true)?),
                                "nth-child" | "nth-last-child" | "nth-of-type"
                                | "nth-last-of-type" => {
                                    let of = args.iter().position(|t| matches!(t,Token::Ident(s) if s.eq_ignore_ascii_case("of")));
                                    if of.is_some() && name.contains("of-type") {
                                        return Err(SelectorError);
                                    }
                                    let (formula, filter) = if let Some(of) = of {
                                        (
                                            &args[..of],
                                            parse_list_at(&args[of + 1..], depth + 1, false)?,
                                        )
                                    } else {
                                        (args, Vec::new())
                                    };
                                    let (a, b) = parse_nth(formula)?;
                                    Simple::Nth {
                                        a,
                                        b,
                                        from_end: name.contains("last"),
                                        of_type: name.contains("of-type"),
                                        filter,
                                    }
                                }
                                _ => return Err(SelectorError),
                            }
                        }
                        _ => return Err(SelectorError),
                    }
                }
                _ => break,
            };
            specificity = specificity.add(match &simple {
                Simple::Id(_) => Specificity(1, 0, 0),
                Simple::Not(list) | Simple::Is(list) => {
                    list.iter().map(|s| s.specificity).max().unwrap_or_default()
                }
                Simple::Where(_) => Specificity::default(),
                Simple::Nth { filter, .. } => Specificity(0, 1, 0).add(
                    filter
                        .iter()
                        .map(|s| s.specificity)
                        .max()
                        .unwrap_or_default(),
                ),
                _ => Specificity(0, 1, 0),
            });
            compound.push(simple);
        }
        if compound.is_empty() {
            return Err(SelectorError);
        }
        compounds.push(compound);
        let mut whitespace = false;
        while tokens.get(index) == Some(&Token::Whitespace) {
            index += 1;
            whitespace = true;
        }
        if index == tokens.len() {
            break;
        }
        if pseudo_element.is_some() {
            return Err(SelectorError);
        }
        let combinator = match tokens.get(index) {
            Some(Token::Delim('>')) => {
                index += 1;
                Combinator::Child
            }
            Some(Token::Delim('+')) => {
                index += 1;
                Combinator::Adjacent
            }
            Some(Token::Delim('~')) => {
                index += 1;
                Combinator::Sibling
            }
            _ if whitespace => Combinator::Descendant,
            _ => return Err(SelectorError),
        };
        while tokens.get(index) == Some(&Token::Whitespace) {
            index += 1;
        }
        if index == tokens.len() {
            return Err(SelectorError);
        }
        combinators.push(combinator);
    }
    if compounds.is_empty() || combinators.len() + 1 != compounds.len() {
        return Err(SelectorError);
    }
    Ok(Selector {
        compounds,
        combinators,
        specificity,
        pseudo_element,
    })
}

fn nth_except(b: i64, from_end: bool) -> Selector {
    // :only-child = neither second-or-later nor second-or-earlier.
    Selector {
        compounds: vec![vec![Simple::Nth {
            a: 1,
            b: b + 1,
            from_end,
            of_type: false,
            filter: Vec::new(),
        }]],
        combinators: Vec::new(),
        specificity: Specificity(0, 1, 0),
        pseudo_element: None,
    }
}

fn parse_attribute(tokens: &[Token]) -> Result<AttrSelector, SelectorError> {
    let tokens: Vec<&Token> = tokens.iter().filter(|t| **t != Token::Whitespace).collect();
    let Some(Token::Ident(name)) = tokens.first().copied() else {
        return Err(SelectorError);
    };
    if tokens.len() == 1 {
        return Ok(AttrSelector {
            name: name.clone(),
            op: AttrOp::Exists,
            value: String::new(),
            ignore_case: None,
        });
    }
    let (op, index) = match tokens.get(1).copied() {
        Some(Token::Delim('=')) => (AttrOp::Equals, 2),
        Some(Token::Delim(ch)) if tokens.get(2).copied() == Some(&Token::Delim('=')) => (
            match ch {
                '~' => AttrOp::Includes,
                '|' => AttrOp::Dash,
                '^' => AttrOp::Prefix,
                '$' => AttrOp::Suffix,
                '*' => AttrOp::Substring,
                _ => return Err(SelectorError),
            },
            3,
        ),
        _ => return Err(SelectorError),
    };
    let value = match tokens.get(index).copied() {
        Some(Token::Ident(v) | Token::QuotedString(v)) => v.clone(),
        _ => return Err(SelectorError),
    };
    let ignore_case = match tokens.get(index + 1).copied() {
        Some(Token::Ident(flag)) if flag.eq_ignore_ascii_case("i") => Some(true),
        Some(Token::Ident(flag)) if flag.eq_ignore_ascii_case("s") => Some(false),
        None => None,
        _ => return Err(SelectorError),
    };
    if tokens.len() > index + 1 + usize::from(ignore_case.is_some()) {
        return Err(SelectorError);
    }
    Ok(AttrSelector {
        name: name.clone(),
        op,
        value,
        ignore_case,
    })
}

fn parse_nth(tokens: &[Token]) -> Result<(i64, i64), SelectorError> {
    let mut text = String::new();
    let mut saw_n = false;
    let mut previous_sign = false;
    for token in tokens {
        if *token == Token::Whitespace {
            continue;
        }
        if matches!(token, Token::Number(n) if *n >= 0.0) && saw_n && !previous_sign {
            text.push('+');
        }
        let part = serialize(std::slice::from_ref(token)).to_ascii_lowercase();
        saw_n |= part.contains('n');
        previous_sign = matches!(token, Token::Delim('+' | '-'));
        text.push_str(&part);
    }
    if text == "odd" {
        return Ok((2, 1));
    }
    if text == "even" {
        return Ok((2, 0));
    }
    if let Some((left, right)) = text.split_once('n') {
        let a = match left {
            "" | "+" => 1,
            "-" => -1,
            s => s.parse().map_err(|_| SelectorError)?,
        };
        let b = if right.is_empty() {
            0
        } else {
            right.parse().map_err(|_| SelectorError)?
        };
        if !(-1_000_000..=1_000_000).contains(&a) || !(-1_000_000..=1_000_000).contains(&b) {
            return Err(SelectorError);
        }
        Ok((a, b))
    } else {
        Ok((0, text.parse().map_err(|_| SelectorError)?))
    }
}

impl Selector {
    pub fn matches(&self, doc: &Document, node: NodeId, context: MatchContext) -> bool {
        if node >= doc.node_count()
            || doc.node_count() == 0
            || self.pseudo_element != context.pseudo_element
        {
            return false;
        }
        let mut budget = 20_000;
        self.match_part(
            doc,
            node,
            self.compounds.len().saturating_sub(1),
            context,
            &mut budget,
        )
    }

    fn match_part(
        &self,
        doc: &Document,
        node: NodeId,
        index: usize,
        ctx: MatchContext,
        budget: &mut usize,
    ) -> bool {
        if *budget == 0 {
            return false;
        }
        *budget -= 1;
        if !matches!(doc.get(node).data, NodeData::Element(_))
            || !self.compounds[index]
                .iter()
                .all(|s| match_simple(s, doc, node, ctx, budget))
        {
            return false;
        }
        if index == 0 {
            return true;
        }
        match self.combinators[index - 1] {
            Combinator::Child => parent_element(doc, node)
                .is_some_and(|id| self.match_part(doc, id, index - 1, ctx, budget)),
            Combinator::Adjacent => previous_element(doc, node)
                .is_some_and(|id| self.match_part(doc, id, index - 1, ctx, budget)),
            combinator => {
                let next = |id| {
                    if combinator == Combinator::Descendant {
                        parent_element(doc, id)
                    } else {
                        previous_element(doc, id)
                    }
                };
                let mut cursor = next(node);
                while let Some(id) = cursor {
                    if *budget == 0 {
                        break;
                    }
                    if self.match_part(doc, id, index - 1, ctx, budget) {
                        return true;
                    }
                    cursor = next(id);
                }
                false
            }
        }
    }
}

fn parent_element(doc: &Document, id: NodeId) -> Option<NodeId> {
    doc.get(id)
        .parent
        .filter(|&p| matches!(doc.get(p).data, NodeData::Element(_)))
}

fn previous_element(doc: &Document, id: NodeId) -> Option<NodeId> {
    let parent = doc.get(id).parent?;
    let siblings = &doc.get(parent).children;
    let index = siblings.iter().position(|&n| n == id)?;
    siblings[..index]
        .iter()
        .rev()
        .find(|&&n| matches!(doc.get(n).data, NodeData::Element(_)))
        .copied()
}

fn match_simple(
    simple: &Simple,
    doc: &Document,
    node: NodeId,
    ctx: MatchContext,
    budget: &mut usize,
) -> bool {
    let NodeData::Element(el) = &doc.get(node).data else {
        return false;
    };
    match simple {
        Simple::Universal => true,
        Simple::Type(tag) => if el.namespace == Namespace::Html { el.tag_name.eq_ignore_ascii_case(tag) } else { el.tag_name == *tag },
        Simple::Id(id) => doc.get_attribute(node,"id") == Some(id.as_str()),
        Simple::Class(class) => doc.get_attribute(node,"class").is_some_and(|v| v.split(crate::token::css_whitespace).any(|c| c == class)),
        Simple::Attribute(attr) => {
            let name = if el.namespace == Namespace::Html { attr.name.to_ascii_lowercase() } else { attr.name.clone() };
            let Some(value) = doc.get_attribute(node, &name) else { return false };
            if attr.op == AttrOp::Exists { return true; }
            let insensitive = attr.ignore_case.unwrap_or(el.namespace == Namespace::Html && matches!(name.as_str(), "type" | "dir" | "rel" | "align" | "method" | "enctype"));
            let (value, needle) = if insensitive { (value.to_ascii_lowercase(), attr.value.to_ascii_lowercase()) } else { (value.to_owned(), attr.value.clone()) };
            match attr.op {
                AttrOp::Exists => true, AttrOp::Equals => value == needle,
                AttrOp::Includes => !needle.is_empty() && value.split(crate::token::css_whitespace).any(|v| v == needle),
                AttrOp::Dash => value == needle || value.starts_with(&(needle + "-")),
                AttrOp::Prefix => !needle.is_empty() && value.starts_with(&needle),
                AttrOp::Suffix => !needle.is_empty() && value.ends_with(&needle),
                AttrOp::Substring => !needle.is_empty() && value.contains(&needle),
            }
        }
        Simple::Root => doc.document_element() == Some(node),
        Simple::Empty => !doc.get(node).children.iter().any(|&id| matches!(&doc.get(id).data, NodeData::Element(_) | NodeData::Text(_) if !matches!(&doc.get(id).data, NodeData::Text(t) if t.is_empty()))),
        Simple::Link => matches!(el.tag_name.as_str(), "a" | "area" | "link") && doc.get_attribute(node,"href").is_some(),
        Simple::Visited => false,
        Simple::Hover => ctx.hovered == Some(node), Simple::Focus => ctx.focused == Some(node), Simple::Active => ctx.active == Some(node),
        Simple::Checked => doc.get_attribute(node,"checked").is_some() || doc.get_attribute(node,"selected").is_some(),
        Simple::Disabled => doc.get_attribute(node,"disabled").is_some(),
        Simple::Enabled => matches!(el.tag_name.as_str(), "input" | "button" | "select" | "textarea" | "option" | "optgroup") && doc.get_attribute(node,"disabled").is_none(),
        Simple::Nth { a,b,from_end,of_type,filter } => {
            let Some(parent) = doc.get(node).parent else { return false };
            let siblings: Vec<NodeId> = doc.get(parent).children.iter().copied().filter(|&id| matches!(&doc.get(id).data, NodeData::Element(other) if !of_type || (other.tag_name == el.tag_name && other.namespace == el.namespace))).filter(|&id| filter.is_empty() || filter.iter().any(|s| s.pseudo_element.is_none() && s.match_part(doc,id,s.compounds.len()-1,ctx,budget))).collect();
            let Some(position) = siblings.iter().position(|&id| id == node) else { return false };
            let index = if *from_end { siblings.len()-position } else { position+1 } as i64;
            let difference = index.saturating_sub(*b);
            if *a == 0 { difference == 0 } else { difference % a == 0 && difference / a >= 0 }
        }
        Simple::Not(list) => !list.iter().any(|s| s.pseudo_element.is_none() && s.match_part(doc,node,s.compounds.len()-1,ctx,budget)),
        Simple::Is(list) | Simple::Where(list) => list.iter().any(|s| s.pseudo_element.is_none() && s.match_part(doc,node,s.compounds.len()-1,ctx,budget)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Document {
        let url = "https://example.com/".parse().unwrap();
        html::parse_full(b"<ul id='list'><li class='a' data-x='Hello world'>a</li>text<!--x--><li id='middle' class='b'>b</li><li class='a' lang='en-US'>c</li></ul><a id='link' href='/'>link</a>", &url, html::ParseOpts::default())
    }

    #[test]
    fn structure_specificity_and_rejection() {
        let selector = &parse_selector_list("#list > li.a:not(.b):nth-child(2n+1)").unwrap()[0];
        assert_eq!(selector.specificity, Specificity(1, 3, 1));
        let doc = fixture();
        let elements = doc.get_elements_by_tag_name("li");
        assert!(selector.matches(&doc, elements[0], MatchContext::default()));
        assert!(!selector.matches(&doc, elements[1], MatchContext::default()));
        assert!(selector.matches(&doc, elements[2], MatchContext::default()));
        assert!(parse_selector_list("p, ::unknown").is_err());
        assert!(parse_selector_list("li >").is_err());
        assert_eq!(
            parse_selector_list(":where(#list) li:is(.a,#middle)").unwrap()[0].specificity,
            Specificity(1, 0, 1)
        );
    }

    #[test]
    fn siblings_attributes_and_link_privacy() {
        let doc = fixture();
        let middle = doc.get_element_by_id("middle").unwrap();
        for text in [
            "li.a + #middle",
            "li.a ~ li.b",
            "ul li:nth-child(2)",
            "li:not(:last-child):not(:first-child)",
        ] {
            assert!(
                parse_selector_list(text).unwrap()[0].matches(
                    &doc,
                    middle,
                    MatchContext::default()
                ),
                "{text}"
            );
        }
        let first = doc.get_elements_by_tag_name("li")[0];
        for text in [
            "[data-x^='hello' i]",
            "[data-x~='world']",
            "[data-x$='world']",
            "[data-x*='llo']",
        ] {
            assert!(parse_selector_list(text).unwrap()[0].matches(
                &doc,
                first,
                MatchContext::default()
            ));
        }
        let link = doc.get_element_by_id("link").unwrap();
        assert!(parse_selector_list(":link").unwrap()[0].matches(
            &doc,
            link,
            MatchContext::default()
        ));
        assert!(!parse_selector_list(":visited").unwrap()[0].matches(
            &doc,
            link,
            MatchContext::default()
        ));
    }
}
