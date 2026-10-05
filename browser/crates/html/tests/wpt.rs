//! WPT-derived conformance harness (plan/13).
//!
//! - `wpt/entities.test`: real html5lib-tests tokenizer file (80 tests).
//! - `wpt/local.dat`: hand-authored tree-construction suite in the exact
//!   html5lib `.dat` format (upstream moved tree tests into WPT html files,
//!   whose harness needs JS; the format stays compatible so real files can
//!   be dropped in later).
//!
//! Tokenizer comparison joins adjacent `Characters` runs first, like the
//! html5ever driver does.

use std::collections::BTreeMap;
use std::path::Path;

use html::{Token, Tokenizer};

// ---------- tokenizer .test ----------

#[derive(Debug)]
struct TokTest {
    description: String,
    input: String,
    initial_state: String,
    expected: Vec<ExpectedToken>,
}

#[derive(Debug, PartialEq)]
enum ExpectedToken {
    Doctype {
        name: Option<String>,
        public: Option<String>,
        system: Option<String>,
    },
    StartTag {
        name: String,
        attributes: BTreeMap<String, String>,
    },
    EndTag {
        name: String,
    },
    Comment(String),
    Characters(String),
}

fn load_tokenizer_tests(path: &Path) -> Vec<TokTest> {
    let text = std::fs::read_to_string(path).expect("tokenizer .test file exists");
    let json: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
    json["tests"]
        .as_array()
        .expect("tests array")
        .iter()
        .map(|test| {
            let expected = test["output"]
                .as_array()
                .expect("output array")
                .iter()
                .map(|tok| {
                    let kind = tok[0].as_str().expect("token kind");
                    match kind {
                        "Doctype" => ExpectedToken::Doctype {
                            name: tok.get(1).and_then(|v| v.as_str()).map(str::to_owned),
                            public: tok.get(2).and_then(|v| v.as_str()).map(str::to_owned),
                            system: tok.get(3).and_then(|v| v.as_str()).map(str::to_owned),
                        },
                        "StartTag" => {
                            let attributes = tok
                                .get(2)
                                .and_then(|v| v.as_object())
                                .map(|map| {
                                    map.iter()
                                        .map(|(k, v)| {
                                            (k.clone(), v.as_str().unwrap_or("").to_owned())
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();
                            ExpectedToken::StartTag {
                                name: tok[1].as_str().expect("tag name").to_owned(),
                                attributes,
                            }
                        }
                        "EndTag" => ExpectedToken::EndTag {
                            name: tok[1].as_str().expect("tag name").to_owned(),
                        },
                        "Comment" => ExpectedToken::Comment(
                            tok[1].as_str().expect("comment text").to_owned(),
                        ),
                        "Character" => ExpectedToken::Characters(
                            tok[1].as_str().expect("char text").to_owned(),
                        ),
                        other => panic!("unknown token kind {other}"),
                    }
                })
                .collect();
            TokTest {
                description: test["description"].as_str().unwrap_or("").to_owned(),
                input: test["input"].as_str().expect("input").to_owned(),
                initial_state: test
                    .get("initialStates")
                    .and_then(|v| v.as_array())
                    .and_then(|v| v.last())
                    .and_then(|v| v.as_str())
                    .unwrap_or("Data state")
                    .to_owned(),
                expected,
            }
        })
        .collect()
}

fn tokenize_for_test(input: &str, initial_state: &str) -> Vec<ExpectedToken> {
    let mut tokenizer = Tokenizer::new();
    match initial_state {
        "RCDATA state" => tokenizer.set_state_for_tag("title"),
        "RAWTEXT state" => tokenizer.set_state_for_tag("style"),
        "Script data state" => tokenizer.set_state_for_tag("script"),
        "PLAINTEXT state" => tokenizer.set_state_for_tag("plaintext"),
        "CDATA state" => tokenizer.set_cdata_allowed(true),
        _ => {}
    }
    tokenizer.feed(input);
    // Join adjacent character runs before comparing (html5ever convention).
    let mut out: Vec<ExpectedToken> = Vec::new();
    loop {
        match tokenizer.next_token() {
            Token::Eof => break,
            Token::Characters(text) => {
                if let Some(ExpectedToken::Characters(existing)) = out.last_mut() {
                    existing.push_str(&text);
                } else {
                    out.push(ExpectedToken::Characters(text));
                }
            }
            Token::StartTag {
                name, attributes, ..
            } => out.push(ExpectedToken::StartTag {
                name,
                attributes: attributes.into_iter().collect(),
            }),
            Token::EndTag { name } => out.push(ExpectedToken::EndTag { name }),
            Token::Comment(text) => out.push(ExpectedToken::Comment(text)),
            Token::Doctype {
                name,
                public_id,
                system_id,
                ..
            } => out.push(ExpectedToken::Doctype {
                name,
                public: public_id,
                system: system_id,
            }),
        }
    }
    out
}

#[test]
fn wpt_tokenizer_entities() {
    let tests = load_tokenizer_tests(Path::new("tests/wpt/entities.test"));
    assert!(!tests.is_empty(), "entities.test loaded");
    let mut failures = Vec::new();
    for test in &tests {
        let actual = tokenize_for_test(&test.input, &test.initial_state);
        if actual != test.expected {
            failures.push(format!(
                "--- {}\ninput: {:?}\nexpected: {:#?}\nactual:   {:#?}",
                test.description, test.input, test.expected, actual
            ));
        }
    }
    if !failures.is_empty() {
        panic!(
            "{}/{} tokenizer tests failed:\n{}",
            failures.len(),
            tests.len(),
            failures.join("\n")
        );
    }
}

// ---------- tree-construction .dat ----------

struct TreeTest {
    data: String,
    expect_errors: bool,
    expected: String,
    scripting: bool,
    fragment_context: Option<String>,
}

fn load_tree_tests(path: &Path) -> Vec<TreeTest> {
    let text = std::fs::read_to_string(path).expect(".dat file exists");
    let mut tests = Vec::new();
    let mut lines = text.lines().peekable();
    while lines.peek().is_some() {
        // Skip blank lines between tests.
        while lines.peek().is_some_and(|line| line.trim().is_empty()) {
            lines.next();
        }
        if lines.peek().is_none() {
            break;
        }
        expect_directive(&mut lines, "#data");
        let mut data = String::new();
        while let Some(line) = lines.peek() {
            if line.starts_with('#') {
                break;
            }
            data.push_str(lines.next().expect("peeked"));
            data.push('\n');
        }
        // The trailing newline after the last data line is not input... but
        // distinguishing "ends with newline" matters. html5lib convention:
        // strip exactly one trailing newline (the format's own separator).
        if data.ends_with('\n') {
            data.pop();
        }
        expect_directive(&mut lines, "#errors");
        let mut errors = String::new();
        while let Some(line) = lines.peek() {
            if line.starts_with('#') {
                break;
            }
            errors.push_str(lines.next().expect("peeked"));
            errors.push('\n');
        }
        let mut scripting = true;
        let mut fragment_context = None;
        // Optional directives before #document.
        loop {
            match lines.peek().map(|line| line.to_string()) {
                Some(directive) if directive == "#script-off" => {
                    scripting = false;
                    lines.next();
                }
                Some(directive) if directive == "#script-on" => {
                    scripting = true;
                    lines.next();
                }
                Some(directive) if directive.starts_with("#document-fragment") => {
                    let context = directive
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("div")
                        .to_owned();
                    fragment_context = Some(context);
                    lines.next();
                }
                _ => break,
            }
        }
        expect_directive(&mut lines, "#document");
        let mut expected = String::new();
        while let Some(line) = lines.peek() {
            // Tree lines always start with `|`; a new `#data` starts the
            // next test. (Text content never starts a line: it is prefixed
            // with `| "` by the serializer.)
            if *line == "#data" {
                break;
            }
            expected.push_str(lines.next().expect("peeked"));
            expected.push('\n');
        }
        tests.push(TreeTest {
            data,
            expect_errors: !errors.trim().is_empty(),
            expected,
            scripting,
            fragment_context,
        });
    }
    tests
}

fn expect_directive(lines: &mut std::iter::Peekable<std::str::Lines<'_>>, directive: &str) {
    let line = lines
        .next()
        .unwrap_or_else(|| panic!("expected {directive}"));
    assert_eq!(line.trim(), directive, "expected directive {directive}");
}

fn run_tree_test(index: usize, test: &TreeTest) -> Option<String> {
    let url = url::Url::parse("https://example.com/").unwrap();
    let opts = html::ParseOpts {
        scripting_enabled: test.scripting,
        ..html::ParseOpts::default()
    };
    let doc = if let Some(context) = &test.fragment_context {
        html::parse_fragment(test.data.as_bytes(), &url, context, opts)
    } else {
        html::parse_full(test.data.as_bytes(), &url, opts)
    };
    let mut problems = Vec::new();
    if test.expect_errors != !doc.errors.is_empty() {
        problems.push(format!(
            "error presence mismatch: expected errors={} got {} ({:?})",
            test.expect_errors,
            !doc.errors.is_empty(),
            doc.errors.iter().map(|e| e.kind).collect::<Vec<_>>()
        ));
    }
    let actual = doc.serialize_html5lib();
    if actual.trim_end() != test.expected.trim_end() {
        problems.push(format!(
            "tree mismatch:\nexpected:\n{}\nactual:\n{}",
            test.expected, actual
        ));
    }
    if problems.is_empty() {
        None
    } else {
        Some(format!(
            "--- tree test #{index} input {:?}:\n{}",
            test.data,
            problems.join("\n")
        ))
    }
}

#[test]
fn wpt_tree_local_suite() {
    let tests = load_tree_tests(Path::new("tests/wpt/local.dat"));
    assert!(!tests.is_empty(), "local.dat loaded");
    let mut failures = Vec::new();
    for (index, test) in tests.iter().enumerate() {
        if let Some(problem) = run_tree_test(index, test) {
            failures.push(problem);
        }
    }
    if !failures.is_empty() {
        panic!(
            "{}/{} tree tests failed:\n{}",
            failures.len(),
            tests.len(),
            failures.join("\n")
        );
    }
}
