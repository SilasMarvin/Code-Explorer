//! Walks a function body in evaluation order, producing call sites and the IR
//! control-flow tree over them.

use std::collections::{BTreeMap, HashMap};

use codeviz_ir::{Arm, CallKey, ExitKind, Mode, Step};
use tree_sitter::Node;

use crate::{
    extract::{
        CallTarget, Extractor, File, Owner, RawCall, RawUnit, Receiver, ReceiverRoot, RouteHint,
        UnitKind,
    },
    hash::content_hash,
    syntax::{
        callee_name, is_constructor, label, location, path_segments, receiver_type_path, start,
        text,
    },
};

pub(crate) struct Context<'a> {
    /// Id path of the unit being built.
    pub(crate) path: &'a [String],
    pub(crate) module: &'a [String],
    pub(crate) owner: Option<&'a Owner>,
    /// A `Result` return type makes tail calls propagate errors.
    pub(crate) returns_result: bool,
    /// The function's or closure's parameter list, for declared binding types.
    pub(crate) parameters: Option<Node<'a>>,
    /// Type parameters and where clause, for the traits generic parameters dispatch to.
    pub(crate) generics: [Option<Node<'a>>; 2],
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
/// Callees that run a closure or function argument concurrently.
const SPAWNERS: [&str; 4] = ["spawn", "spawn_blocking", "spawn_local", "spawn_on"];
/// Callees that keep a closure or function argument to invoke later. Anything else
/// is assumed to call its argument during the call (`map`, `and_then`, `for_each`).
const REGISTRARS: [&str; 16] = [
    "route",
    "route_service",
    "get",
    "post",
    "put",
    "patch",
    "delete",
    "head",
    "options",
    "any",
    "on",
    "subscribe",
    "register",
    "add_listener",
    "connect",
    "set_handler",
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
        async_blocks: 0,
        bindings: HashMap::new(),
        stored_closures: HashMap::new(),
        generics: HashMap::new(),
        call_nodes: HashMap::new(),
    };
    for generics in cx.generics.into_iter().flatten() {
        builder.generics(generics);
    }
    if let Some(parameters) = cx.parameters {
        builder.parameters(parameters);
    }
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
    async_blocks: usize,
    /// Binding name → what its value is known to be. Scopes are ignored: the last
    /// binding of a name wins.
    bindings: HashMap<String, ReceiverRoot>,
    /// `let f = || ..` → the closure's unit path, so `f()` resolves.
    stored_closures: HashMap<String, Vec<String>>,
    /// Generic parameter → the trait bound it dispatches through.
    generics: HashMap<String, Vec<String>>,
    /// Call expression node id → index into `calls`, for receivers that are calls.
    call_nodes: HashMap<usize, usize>,
}

impl Builder<'_, '_, '_> {
    fn source(&self) -> &[u8] {
        self.file.source
    }

    fn parameters(&mut self, parameters: Node<'_>) {
        let source = self.file.source;
        let mut cursor = parameters.walk();
        let params: Vec<Node<'_>> = parameters.named_children(&mut cursor).collect();
        for param in params {
            if param.kind() != "parameter" {
                continue;
            }
            let (Some(name), Some(ty)) = (
                param
                    .child_by_field_name("pattern")
                    .and_then(|p| binding_name(p, source)),
                param
                    .child_by_field_name("type")
                    .and_then(|t| receiver_type_path(t, source)),
            ) else {
                continue;
            };
            let path = self.substitute(ty);
            self.bindings.insert(
                name,
                ReceiverRoot::Typed {
                    path,
                    declared: true,
                },
            );
        }
    }

    /// Records `T: Trait` from type parameters and where clauses.
    fn generics(&mut self, node: Node<'_>) {
        let source = self.file.source;
        let mut cursor = node.walk();
        let children: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
        for child in children {
            let (name, bounds) = match child.kind() {
                "type_parameter" => (
                    child.child_by_field_name("name"),
                    child.child_by_field_name("bounds"),
                ),
                "where_predicate" => (
                    child.child_by_field_name("left"),
                    child.child_by_field_name("bounds"),
                ),
                _ => continue,
            };
            let (Some(name), Some(bounds)) = (name, bounds) else {
                continue;
            };
            let mut cursor = bounds.walk();
            let first_trait = bounds
                .named_children(&mut cursor)
                .find_map(|bound| receiver_type_path(bound, source));
            if let Some(bound) = first_trait {
                self.generics.insert(text(name, source).to_owned(), bound);
            }
        }
    }

    /// A generic parameter's type is its trait bound.
    fn substitute(&self, ty: Vec<String>) -> Vec<String> {
        match ty.as_slice() {
            [single] => self.generics.get(single).cloned().unwrap_or(ty),
            _ => ty,
        }
    }

    /// Records the type of `let name: Type = ..`, `let name = Type { .. }` and
    /// `let name = Type::constructor(..)`.
    fn let_binding(&mut self, declaration: Node<'_>) {
        let source = self.file.source;
        let Some(name) = declaration
            .child_by_field_name("pattern")
            .and_then(|p| binding_name(p, source))
        else {
            return;
        };
        let declared = declaration
            .child_by_field_name("type")
            .and_then(|t| receiver_type_path(t, source));
        if let Some(ty) = declared {
            let path = self.substitute(ty);
            self.bindings.insert(
                name,
                ReceiverRoot::Typed {
                    path,
                    declared: true,
                },
            );
            return;
        }
        let Some(value) = declaration.child_by_field_name("value") else {
            return;
        };
        if value.kind() == "closure_expression" && self.closures > 0 {
            // `expr` has just built this closure's unit.
            let mut path = self.cx.path.to_vec();
            path.push(format!("{{closure#{}}}", self.closures - 1));
            self.stored_closures.insert(name, path);
            return;
        }
        let known = match value.kind() {
            "struct_expression" => value
                .child_by_field_name("name")
                .and_then(|n| path_segments(n, source))
                .map(|path| ReceiverRoot::Typed {
                    path,
                    declared: true,
                }),
            _ => self
                .receiver(value)
                .filter(|r| r.fields.is_empty())
                .map(|r| r.root),
        };
        if let Some(root) = known {
            self.bindings.insert(name, root);
        }
    }

    /// What a method is called on, when declarations say.
    fn receiver(&self, node: Node<'_>) -> Option<Receiver> {
        match node.kind() {
            "self" => Some(Receiver {
                root: ReceiverRoot::SelfValue,
                fields: Vec::new(),
            }),
            "identifier" => Some(Receiver {
                root: self.bindings.get(text(node, self.source()))?.clone(),
                fields: Vec::new(),
            }),
            "call_expression" => Some(Receiver {
                root: ReceiverRoot::Returned {
                    call: *self.call_nodes.get(&node.id())?,
                    tried: false,
                },
                fields: Vec::new(),
            }),
            "await_expression" => self.receiver(node.named_child(0)?),
            "try_expression" => {
                let mut receiver = self.receiver(node.named_child(0)?)?;
                if let ReceiverRoot::Returned { tried, .. } = &mut receiver.root {
                    *tried = true;
                }
                Some(receiver)
            }
            "field_expression" => {
                let mut receiver = self.receiver(node.child_by_field_name("value")?)?;
                let field = node.child_by_field_name("field")?;
                receiver.fields.push(text(field, self.source()).to_owned());
                Some(receiver)
            }
            "parenthesized_expression" | "reference_expression" | "unary_expression" => {
                let last = node.named_child_count().checked_sub(1)?;
                self.receiver(node.named_child(u32::try_from(last).ok()?)?)
            }
            _ => None,
        }
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
            if matches!(steps.last(), Some(Step::Exit { .. })) {
                // Anything after an exit in the same block is unreachable.
                break;
            }
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
                    self.let_binding(child);
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
        // `match never {}` cannot be reached; a single arm always runs.
        match arms.len() {
            0 => {}
            1 => steps.extend(arms.into_iter().flat_map(|a| a.body)),
            _ => steps.push(Step::Branch { guard, arms }),
        }
    }

    fn return_expression(&mut self, node: Node<'_>, steps: &mut Vec<Step>) {
        let value = node.named_child(0);
        if let Some(error) = value.and_then(|v| self.err_constructor(v)) {
            self.expr(error, false, steps);
            steps.push(exit(
                ExitKind::Error,
                Some(error_label(error, self.source())),
            ));
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
        // The `?` applies to the call it wraps (through `.await` and parentheses);
        // for macro-recovered calls, the last call recorded that is not a handoff.
        let wrapped = std::iter::successors(Some(inner), |n| match n.kind() {
            "await_expression" | "parenthesized_expression" => n.named_child(0),
            _ => None,
        })
        .last()
        .and_then(|n| self.call_nodes.get(&n.id()).copied())
        .filter(|&index| index >= before);
        let last_direct = wrapped.or_else(|| {
            self.calls[before..]
                .iter()
                .rposition(|call| call.mode == Mode::Call && call.via_call.is_none())
                .map(|offset| before + offset)
        });
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
                        receiver: function
                            .child_by_field_name("value")
                            .and_then(|v| self.receiver(v)),
                    })
            }
            _ => path_segments(function, source).map(|segments| match segments.as_slice() {
                [name] if self.stored_closures.contains_key(name) => self
                    .stored_closures
                    .get(name)
                    .map_or(CallTarget::Path(segments.clone()), |path| {
                        CallTarget::Closure(path.clone())
                    }),
                _ => CallTarget::Path(segments),
            }),
        };

        if let Some(CallTarget::Path(segments)) = &target
            && is_constructor(segments)
        {
            if let Some(arguments) = arguments {
                self.expr(arguments, false, steps);
            }
            if tail && segments.last().is_some_and(|s| s == "Err") {
                steps.push(exit(ExitKind::Error, Some(error_label(node, source))));
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
                    "closure_expression" | "async_block" => {
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
        let handoff_mode = handoff_mode(&target);
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
            self.record(written, handoff, handoff_mode, Some(index), arg, steps);
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
        self.call_nodes.insert(node.id(), self.calls.len());
        self.calls.push(RawCall {
            key,
            text,
            target,
            mode,
            via_call,
            propagates: false,
            location: location(self.file.path, node),
            // A call expression's callee, or the name passed or macro-called itself.
            name_at: callee_name(node.child_by_field_name("function").unwrap_or(node)).map(start),
        });
        self.calls.len() - 1
    }

    /// Builds a closure or async block as its own unit, parented to this one.
    fn closure(&mut self, node: Node<'_>) -> Vec<String> {
        let mut path = self.cx.path.to_vec();
        let body = if node.kind() == "async_block" {
            path.push(format!("{{async#{}}}", self.async_blocks));
            self.async_blocks += 1;
            node.named_children(&mut node.walk())
                .find(|c| c.kind() == "block")
        } else {
            path.push(format!("{{closure#{}}}", self.closures));
            self.closures += 1;
            node.child_by_field_name("body")
        };
        let Some(body) = body else {
            return path;
        };
        let cx = Context {
            path: &path,
            module: self.cx.module,
            owner: self.cx.owner,
            returns_result: false,
            parameters: node.child_by_field_name("parameters"),
            generics: [None, None],
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
            name_at: None,
            content_hash: Some(content_hash(node, self.file.source)),
            doc: None,
            attributes: BTreeMap::new(),
            calls: built.calls,
            body: Some(built.steps),
            returns: None,
            returns_ok: None,
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
                let receiver = before(3).filter(|t| t.kind() == "self").map(|_| Receiver {
                    root: ReceiverRoot::SelfValue,
                    fields: Vec::new(),
                });
                CallTarget::Method { name, receiver }
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

/// How a callee treats a closure or function passed to it, judged by its name.
fn handoff_mode(callee: &CallTarget) -> Mode {
    let name = match callee {
        CallTarget::Path(segments) | CallTarget::Reference(segments) => {
            segments.last().map_or("", String::as_str)
        }
        CallTarget::Method { name, .. } => name.as_str(),
        CallTarget::Closure(_) => "",
    };
    if SPAWNERS.contains(&name) {
        Mode::Spawn
    } else if REGISTRARS.contains(&name) {
        Mode::Register
    } else {
        Mode::Call
    }
}

/// `Err(AppError::NotFound("user".into()))` → `AppError::NotFound("user")`: the error
/// value, without the `Err` wrapper or a trailing conversion.
fn error_label(err: Node<'_>, source: &[u8]) -> String {
    let value = err
        .child_by_field_name("arguments")
        .and_then(|a| a.named_child(0))
        .map_or_else(|| label(err, source), |v| label(v, source));
    let mut value = value.as_str();
    for conversion in [".into()", ".to_string()", ".to_owned()"] {
        value = value.strip_suffix(conversion).unwrap_or(value);
    }
    value.replace(".into())", ")")
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

/// `x`, `mut x` → `x`. Destructuring patterns bind no single name.
fn binding_name(pattern: Node<'_>, source: &[u8]) -> Option<String> {
    match pattern.kind() {
        "identifier" => Some(text(pattern, source).to_owned()),
        "mut_pattern" => binding_name(pattern.named_child(0)?, source),
        _ => None,
    }
}
