//! Walks a function body in evaluation order, producing call sites and the IR
//! control-flow tree over them.

use std::collections::{BTreeMap, HashMap};

use codeviz_ir::{Arm, CallKey, ExitKind, Mode, Step};
use tree_sitter::Node;

use crate::{
    extract::{CallTarget, Extractor, File, Owner, RawCall, RawUnit, RouteHint, UnitKind},
    hash::content_hash,
    syntax::{is_constructor, label, location, path_segments, text},
};

pub(crate) struct Context<'a> {
    /// Id path of the unit being built.
    pub(crate) path: &'a [String],
    pub(crate) module: &'a [String],
    pub(crate) owner: Option<&'a Owner>,
    /// A `Result` return type makes tail calls propagate errors.
    pub(crate) returns_result: bool,
}

pub(crate) struct Built {
    pub(crate) calls: Vec<RawCall>,
    pub(crate) steps: Vec<Step>,
}

const PANIC_MACROS: [&str; 4] = ["panic", "unreachable", "todo", "unimplemented"];
const ERROR_MACROS: [&str; 1] = ["bail"];
const ROUTE_METHODS: [&str; 8] = [
    "get", "post", "put", "patch", "delete", "head", "options", "any",
];

pub(crate) fn build(
    ex: &mut Extractor,
    file: &File<'_>,
    cx: &Context<'_>,
    body: Node<'_>,
) -> Built {
    let mut builder = Builder {
        ex,
        file,
        cx,
        calls: Vec::new(),
        occurrences: HashMap::new(),
        closures: 0,
    };
    let steps = builder.block_or_expr(body, true);
    Built {
        calls: builder.calls,
        steps,
    }
}

struct Builder<'a, 'b, 'f> {
    ex: &'a mut Extractor,
    file: &'b File<'f>,
    cx: &'b Context<'b>,
    calls: Vec<RawCall>,
    occurrences: HashMap<String, usize>,
    closures: usize,
}

impl Builder<'_, '_, '_> {
    fn source(&self) -> &[u8] {
        self.file.source
    }

    fn block_or_expr(&mut self, node: Node<'_>, tail: bool) -> Vec<Step> {
        let mut steps = Vec::new();
        if node.kind() == "block" {
            self.block(node, tail, &mut steps);
        } else {
            self.expr(node, tail, &mut steps);
        }
        steps
    }

    /// The final expression of a block (no trailing `;`) is its value.
    fn block(&mut self, node: Node<'_>, tail: bool, steps: &mut Vec<Step>) {
        let mut cursor = node.walk();
        let children: Vec<Node<'_>> = node
            .named_children(&mut cursor)
            .filter(|c| !c.kind().ends_with("comment") && c.kind() != "attribute_item")
            .collect();
        let last = children.len().saturating_sub(1);
        for (index, child) in children.into_iter().enumerate() {
            match child.kind() {
                "expression_statement" => {
                    // Block-like expressions (`match`, `if`) are statements even
                    // without a `;`, and are still the block's value when last.
                    let terminated = child
                        .child(child.child_count().saturating_sub(1))
                        .is_some_and(|t| t.kind() == ";");
                    if let Some(inner) = child.named_child(0) {
                        self.expr(inner, tail && index == last && !terminated, steps);
                    }
                }
                "let_declaration" => {
                    if let Some(value) = child.child_by_field_name("value") {
                        self.expr(value, false, steps);
                    }
                    if let Some(alternative) = child.child_by_field_name("alternative") {
                        let otherwise = self.block_or_expr(alternative, false);
                        steps.push(Step::Branch {
                            guard: Some(label(child, self.source())),
                            arms: vec![arm("matches", Vec::new()), arm("else", otherwise)],
                        });
                    }
                }
                _ if self.ex.item(self.file, child, self.cx.module, None) => {}
                _ => self.expr(child, tail && index == last, steps),
            }
        }
    }

    fn expr(&mut self, node: Node<'_>, tail: bool, steps: &mut Vec<Step>) {
        match node.kind() {
            "call_expression" => self.call(node, tail, steps),
            "macro_invocation" => self.macro_invocation(node, steps),
            "if_expression" => self.if_expression(node, tail, steps),
            "match_expression" => self.match_expression(node, tail, steps),
            "for_expression" => {
                if let Some(value) = node.child_by_field_name("value") {
                    self.expr(value, false, steps);
                }
                let pattern = node
                    .child_by_field_name("pattern")
                    .map(|p| label(p, self.source()))
                    .unwrap_or_default();
                let iterated = node
                    .child_by_field_name("value")
                    .map(|v| label(v, self.source()))
                    .unwrap_or_default();
                let body = self.loop_body(node, None);
                steps.push(Step::Loop {
                    label: Some(format!("for {pattern} in {iterated}")),
                    body,
                });
            }
            "while_expression" => {
                let condition = node.child_by_field_name("condition");
                let guard = condition
                    .map(|c| label(c, self.source()))
                    .unwrap_or_default();
                let body = self.loop_body(node, condition);
                steps.push(Step::Loop {
                    label: Some(format!("while {guard}")),
                    body,
                });
            }
            "loop_expression" => {
                let body = self.loop_body(node, None);
                steps.push(Step::Loop {
                    label: Some("loop".to_owned()),
                    body,
                });
            }
            "return_expression" => self.return_expression(node, steps),
            "try_expression" => self.try_expression(node, steps),
            "closure_expression" => {
                self.closure(node);
            }
            "block" | "async_block" | "unsafe_block" => {
                let inner = if node.kind() == "block" {
                    Some(node)
                } else {
                    node.named_children(&mut node.walk())
                        .find(|c| c.kind() == "block")
                };
                if let Some(inner) = inner {
                    self.block(inner, tail, steps);
                }
            }
            _ => {
                let mut cursor = node.walk();
                let children: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
                for child in children {
                    self.expr(child, false, steps);
                }
            }
        }
    }

    fn loop_body(&mut self, node: Node<'_>, condition: Option<Node<'_>>) -> Vec<Step> {
        let mut body = Vec::new();
        if let Some(condition) = condition {
            self.expr(condition, false, &mut body);
        }
        if let Some(block) = node.child_by_field_name("body") {
            self.block(block, false, &mut body);
        }
        body
    }

    fn if_expression(&mut self, node: Node<'_>, tail: bool, steps: &mut Vec<Step>) {
        let condition = node.child_by_field_name("condition");
        if let Some(condition) = condition {
            self.expr(condition, false, steps);
        }
        let guard = condition.map(|c| label(c, self.source()));
        let then = node
            .child_by_field_name("consequence")
            .map(|c| self.block_or_expr(c, tail))
            .unwrap_or_default();
        let otherwise = node
            .child_by_field_name("alternative")
            .and_then(|clause| clause.named_child(0))
            .map(|alternative| self.block_or_expr(alternative, tail))
            .unwrap_or_default();
        steps.push(Step::Branch {
            guard,
            arms: vec![arm("then", then), arm("else", otherwise)],
        });
    }

    fn match_expression(&mut self, node: Node<'_>, tail: bool, steps: &mut Vec<Step>) {
        let value = node.child_by_field_name("value");
        if let Some(value) = value {
            self.expr(value, false, steps);
        }
        let guard = value.map(|v| format!("match {}", label(v, self.source())));
        let mut arms = Vec::new();
        if let Some(block) = node.child_by_field_name("body") {
            let mut cursor = block.walk();
            let match_arms: Vec<Node<'_>> = block
                .named_children(&mut cursor)
                .filter(|c| c.kind() == "match_arm")
                .collect();
            for match_arm in match_arms {
                let pattern = match_arm
                    .child_by_field_name("pattern")
                    .map(|p| label(p, self.source()))
                    .unwrap_or_default();
                let body = match_arm
                    .child_by_field_name("value")
                    .map(|v| self.block_or_expr(v, tail))
                    .unwrap_or_default();
                arms.push(arm(&pattern, body));
            }
        }
        if arms.len() == 1 {
            arms.push(arm("_", Vec::new()));
        }
        steps.push(Step::Branch { guard, arms });
    }

    fn return_expression(&mut self, node: Node<'_>, steps: &mut Vec<Step>) {
        let value = node.named_child(0);
        if let Some(error) = value.and_then(|v| self.err_constructor(v)) {
            self.expr(error, false, steps);
            steps.push(exit(ExitKind::Error, Some(label(error, self.source()))));
            return;
        }
        if let Some(value) = value {
            self.expr(value, true, steps);
        }
        steps.push(exit(ExitKind::Return, None));
    }

    /// `x?`: the outermost call inside `x` may end this function with its error.
    fn try_expression(&mut self, node: Node<'_>, steps: &mut Vec<Step>) {
        let Some(inner) = node.named_child(0) else {
            return;
        };
        let before = self.calls.len();
        self.expr(inner, false, steps);
        let last_direct = self.calls[before..]
            .iter()
            .rposition(|call| call.mode == Mode::Call)
            .map(|offset| before + offset);
        if let Some(call) = last_direct.and_then(|index| self.calls.get_mut(index)) {
            call.propagates = true;
        } else {
            let source = label(node, self.source());
            steps.push(Step::Branch {
                guard: Some(source.clone()),
                arms: vec![
                    arm("err", vec![exit(ExitKind::Error, Some(source))]),
                    arm("ok", Vec::new()),
                ],
            });
        }
    }

    fn call(&mut self, node: Node<'_>, tail: bool, steps: &mut Vec<Step>) {
        let source = self.file.source;
        let (Some(function), arguments) = (
            node.child_by_field_name("function"),
            node.child_by_field_name("arguments"),
        ) else {
            return;
        };

        let target = match function.kind() {
            "field_expression" => {
                if let Some(receiver) = function.child_by_field_name("value") {
                    self.expr(receiver, false, steps);
                }
                function
                    .child_by_field_name("field")
                    .map(|field| CallTarget::Method {
                        name: text(field, source).to_owned(),
                        on_self: function
                            .child_by_field_name("value")
                            .is_some_and(|v| v.kind() == "self"),
                    })
            }
            _ => path_segments(function, source).map(CallTarget::Path),
        };

        if let Some(CallTarget::Path(segments)) = &target
            && is_constructor(segments)
        {
            if let Some(arguments) = arguments {
                self.expr(arguments, false, steps);
            }
            if tail && segments.last().is_some_and(|s| s == "Err") {
                steps.push(exit(ExitKind::Error, Some(label(node, source))));
            }
            return;
        }

        if matches!(&target, Some(CallTarget::Method { name, .. }) if name == "route") {
            self.route_hint(arguments);
        }

        let mut handoffs = Vec::new();
        if let Some(arguments) = arguments {
            let mut cursor = arguments.walk();
            let args: Vec<Node<'_>> = arguments.named_children(&mut cursor).collect();
            for arg in args {
                match arg.kind() {
                    "closure_expression" => {
                        let path = self.closure(arg);
                        handoffs.push((CallTarget::Closure(path), arg));
                    }
                    "identifier" | "scoped_identifier" => {
                        if let Some(segments) = path_segments(arg, source)
                            && !is_constructor(&segments)
                        {
                            handoffs.push((CallTarget::Reference(segments), arg));
                        }
                    }
                    _ => self.expr(arg, false, steps),
                }
            }
        }

        let Some(target) = target else {
            return;
        };
        let written = match &target {
            CallTarget::Method { name, .. } => format!(".{name}"),
            _ => text(function, source).split_whitespace().collect(),
        };
        let index = self.record(written, target, Mode::Call, None, node, steps);
        if tail
            && self.cx.returns_result
            && let Some(call) = self.calls.get_mut(index)
        {
            call.propagates = true;
        }
        for (handoff, arg) in handoffs {
            let written = match &handoff {
                CallTarget::Closure(path) => path.last().cloned().unwrap_or_default(),
                _ => text(arg, source).to_owned(),
            };
            self.record(written, handoff, Mode::Register, Some(index), arg, steps);
        }
    }

    fn record(
        &mut self,
        text: String,
        target: CallTarget,
        mode: Mode,
        via_call: Option<usize>,
        node: Node<'_>,
        steps: &mut Vec<Step>,
    ) -> usize {
        let occurrence = self.occurrences.entry(text.clone()).or_default();
        let key = CallKey::for_text(&text, *occurrence);
        *occurrence += 1;
        steps.push(Step::Call { call: key.clone() });
        self.calls.push(RawCall {
            key,
            text,
            target,
            mode,
            via_call,
            propagates: false,
            location: location(self.file.path, node),
        });
        self.calls.len() - 1
    }

    fn closure(&mut self, node: Node<'_>) -> Vec<String> {
        let mut path = self.cx.path.to_vec();
        path.push(format!("{{closure#{}}}", self.closures));
        self.closures += 1;
        let Some(body) = node.child_by_field_name("body") else {
            return path;
        };
        let cx = Context {
            path: &path,
            module: self.cx.module,
            owner: self.cx.owner,
            returns_result: false,
        };
        let built = build(self.ex, self.file, &cx, body);
        self.ex.push_unit(RawUnit {
            parent: Some(self.cx.path.to_vec()),
            name: path.last().cloned().unwrap_or_default(),
            path: path.clone(),
            kind: UnitKind::Closure,
            module: self.cx.module.to_vec(),
            owner: self.cx.owner.cloned(),
            location: Some(location(self.file.path, node)),
            content_hash: Some(content_hash(node, self.file.source)),
            doc: None,
            attributes: BTreeMap::new(),
            calls: built.calls,
            body: Some(built.steps),
        });
        path
    }

    fn macro_invocation(&mut self, node: Node<'_>, steps: &mut Vec<Step>) {
        let source = self.file.source;
        let name = node
            .child_by_field_name("macro")
            .and_then(|m| path_segments(m, source))
            .and_then(|segments| segments.last().cloned())
            .unwrap_or_default();
        if PANIC_MACROS.contains(&name.as_str()) {
            steps.push(exit(ExitKind::Panic, Some(label(node, source))));
            return;
        }
        if ERROR_MACROS.contains(&name.as_str()) {
            steps.push(exit(ExitKind::Error, Some(label(node, source))));
            return;
        }
        let mut cursor = node.walk();
        let trees: Vec<Node<'_>> = node
            .named_children(&mut cursor)
            .filter(|c| c.kind() == "token_tree")
            .collect();
        for tree in trees {
            self.macro_calls(tree, steps);
        }
    }

    /// Tree-sitter leaves macro arguments as raw tokens. Recover `a::b(..)` and
    /// `x.m(..)` shapes so calls inside `format!` and friends are not lost.
    fn macro_calls(&mut self, tree: Node<'_>, steps: &mut Vec<Step>) {
        let source = self.file.source;
        let mut cursor = tree.walk();
        let tokens: Vec<Node<'_>> = tree.children(&mut cursor).collect();
        for (index, token) in tokens.iter().enumerate() {
            if token.kind() != "token_tree" {
                continue;
            }
            self.macro_calls(*token, steps);

            let before = |offset: usize| index.checked_sub(offset).and_then(|i| tokens.get(i));
            let Some(callee) = before(1).filter(|c| c.kind() == "identifier") else {
                continue;
            };
            if !text(*token, source).starts_with('(') {
                continue;
            }
            let name = text(*callee, source).to_owned();
            let target = if before(2).is_some_and(|t| t.kind() == ".") {
                let on_self = before(3).is_some_and(|t| t.kind() == "self");
                CallTarget::Method { name, on_self }
            } else {
                let mut segments = vec![name];
                let mut offset = 2;
                while before(offset).is_some_and(|t| t.kind() == "::")
                    && let Some(segment) = before(offset + 1)
                    && matches!(segment.kind(), "identifier" | "self" | "super" | "crate")
                {
                    segments.insert(0, text(*segment, source).to_owned());
                    offset += 2;
                }
                if is_constructor(&segments) {
                    continue;
                }
                CallTarget::Path(segments)
            };
            let written = match &target {
                CallTarget::Method { name, .. } => format!(".{name}"),
                CallTarget::Path(segments) => segments.join("::"),
                CallTarget::Reference(_) | CallTarget::Closure(_) => continue,
            };
            self.record(written, target, Mode::Call, None, *callee, steps);
        }
    }

    /// `.route("/path", get(handler).post(other))`
    fn route_hint(&mut self, arguments: Option<Node<'_>>) {
        let source = self.file.source;
        let Some(arguments) = arguments else {
            return;
        };
        let mut cursor = arguments.walk();
        let args: Vec<Node<'_>> = arguments.named_children(&mut cursor).collect();
        let (Some(path), Some(methods)) = (args.first(), args.get(1)) else {
            return;
        };
        if path.kind() != "string_literal" {
            return;
        }
        let path = text(*path, source).trim_matches('"').to_owned();

        let mut stack = vec![*methods];
        while let Some(node) = stack.pop() {
            let mut cursor = node.walk();
            stack.extend(node.named_children(&mut cursor));
            if node.kind() != "call_expression" {
                continue;
            }
            let method = node
                .child_by_field_name("function")
                .and_then(|f| match f.kind() {
                    "field_expression" => f.child_by_field_name("field"),
                    _ => Some(f),
                });
            let method = method.map(|m| text(m, source)).unwrap_or_default();
            let method = method.rsplit("::").next().unwrap_or_default();
            let handler = node
                .child_by_field_name("arguments")
                .and_then(|a| a.named_child(0))
                .and_then(|h| path_segments(h, source));
            if let Some(handler) = handler
                && ROUTE_METHODS.contains(&method)
            {
                self.ex.push_route(RouteHint {
                    method: method.to_uppercase(),
                    path: path.clone(),
                    handler,
                    module: self.cx.module.to_vec(),
                });
            }
        }
    }

    /// `Err(..)` as a returned value.
    fn err_constructor<'t>(&self, node: Node<'t>) -> Option<Node<'t>> {
        (node.kind() == "call_expression")
            .then(|| node.child_by_field_name("function"))
            .flatten()
            .filter(|f| text(*f, self.file.source) == "Err")
            .map(|_| node)
    }
}

fn arm(label: &str, body: Vec<Step>) -> Arm {
    Arm {
        label: label.to_owned(),
        body,
    }
}

fn exit(kind: ExitKind, label: Option<String>) -> Step {
    Step::Exit { kind, label }
}
