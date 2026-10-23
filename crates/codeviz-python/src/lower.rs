//! Lowers one executable unit (a module body, function or lambda) to call sites, a
//! control-flow body and the entry points it declares.
//!
//! Mapping decisions:
//! - Calls are listed in source order and keyed by it; the body orders them as Python
//!   evaluates them (arguments before the call).
//! - Every `call`-mode invocation propagates, since any Python call may raise, unless it
//!   sits in a `try` body with an `except` clause.
//! - `try` lowers to the try body, then the `finally` body, then a branch between the
//!   `else` path (`ok`) and each `except` clause. This keeps `finally` on every path at
//!   the cost of running it before, rather than after, a handler.
//! - A function passed as an argument is registered with the callee (`via`), or with
//!   the receiver variable when the callee is unknown (`app.add_event_handler(…, f)`).
//! - `asyncio.create_task(f())` and `ensure_future` spawn `f`.
//! - A route decorator (`@router.post("/path")`) registers the function on the router
//!   and declares an `http` entry; other decorators are calls made at definition time,
//!   and the wrappers they return are listed in the entry's `chain`.
//! - An import of an analyzed module runs that module's body.

use std::collections::{BTreeSet, HashMap};

use codeviz_ir::{
    Arm, CallKey, Certainty, Entry, ExitKind, Invocation, Location, Mode, Step, Target, UnitId,
};
use tree_sitter::Node;

use crate::{
    imports::{self, Importer},
    index::{Code, Executable, Index, ScopeId, module_body},
    resolve::{Resolver, Value},
    syntax::{
        collapsed, compact, field, last_name, location, named_children, string_literal, text,
    },
};

const HTTP_METHODS: &[&str] = &["get", "post", "put", "patch", "delete", "head", "options"];
const SPAWNERS: &[&str] = &["create_task", "ensure_future"];
/// Decorators that only mark how a function binds; they are not reported as calls.
const MARKERS: &[&str] = &[
    "classmethod",
    "staticmethod",
    "property",
    "abstractmethod",
    "abc.abstractmethod",
];

/// A call site before keys are assigned.
#[derive(Debug, Clone)]
pub(crate) struct RawCall {
    pub(crate) text: String,
    start: usize,
    pub(crate) targets: Vec<Target>,
    mode: Mode,
    via: Option<UnitId>,
    propagates: bool,
    location: Location,
    /// For an import: the analyzed module it loads.
    pub(crate) loads: Option<String>,
}

/// One executable unit, lowered. Steps refer to calls by provisional index until
/// [`Lowered::finish`] assigns keys.
#[derive(Debug)]
pub(crate) struct Lowered {
    pub(crate) unit: UnitId,
    /// Set for a module body: the module's name.
    pub(crate) module: Option<String>,
    pub(crate) calls: Vec<RawCall>,
    steps: Vec<Step>,
    pub(crate) entries: Vec<Entry>,
    /// Module-level variables things were registered on.
    pub(crate) vias: BTreeSet<UnitId>,
}

pub(crate) fn lower(
    resolver: &Resolver<'_, '_>,
    channels: &BTreeSet<UnitId>,
    executable: &Executable<'_>,
) -> Lowered {
    let index = resolver.index();
    let file = index.scopes[executable.scope].file;
    let mut lowerer = Lowerer {
        resolver,
        index,
        channels,
        file,
        source: &index.parsed(file).source,
        unit: &executable.unit,
        calls: Vec::new(),
        entries: Vec::new(),
        vias: BTreeSet::new(),
        protected: false,
    };
    let scope = executable.scope;
    let (steps, module) = match executable.code {
        Code::Module(root) => (
            lowerer.block(scope, root),
            Some(index.parsed(file).file.module.clone()),
        ),
        Code::Block(body) => (lowerer.block(scope, body), None),
        Code::Expression(body) => {
            let mut steps = Vec::new();
            lowerer.expression(scope, body, &mut steps);
            (steps, None)
        }
    };
    Lowered {
        unit: executable.unit.clone(),
        module,
        calls: lowerer.calls,
        steps,
        entries: lowerer.entries,
        vias: lowerer.vias,
    }
}

struct Lowerer<'a, 'i, 't> {
    resolver: &'a Resolver<'i, 't>,
    index: &'i Index<'t>,
    /// Module-level variables known to have things registered on them.
    channels: &'a BTreeSet<UnitId>,
    file: usize,
    source: &'t str,
    unit: &'a UnitId,
    calls: Vec<RawCall>,
    entries: Vec<Entry>,
    vias: BTreeSet<UnitId>,
    /// Inside a `try` body that has `except` clauses.
    protected: bool,
}

impl Lowerer<'_, '_, '_> {
    fn block(&mut self, scope: ScopeId, block: Node<'_>) -> Vec<Step> {
        let mut steps = Vec::new();
        for statement in named_children(block) {
            self.statement(scope, statement, &mut steps);
        }
        steps
    }

    fn statement(&mut self, scope: ScopeId, node: Node<'_>, out: &mut Vec<Step>) {
        match node.kind() {
            "if_statement" => self.if_statement(scope, node, out),
            "match_statement" => self.match_statement(scope, node, out),
            "for_statement" => self.for_statement(scope, node, out),
            "while_statement" => self.while_statement(scope, node, out),
            "try_statement" => self.try_statement(scope, node, out),
            "with_statement" => {
                for child in named_children(node) {
                    if child.kind() == "block" {
                        out.extend(self.block(scope, child));
                    } else {
                        self.expression(scope, child, out);
                    }
                }
            }
            "return_statement" => {
                self.expression(scope, node, out);
                out.push(Step::Exit {
                    kind: ExitKind::Return,
                    label: None,
                });
            }
            "raise_statement" => {
                self.expression(scope, node, out);
                out.push(Step::Exit {
                    kind: ExitKind::Error,
                    label: Some(self.raised(node)),
                });
            }
            "decorated_definition" => self.decorated(scope, node, out),
            "class_definition" => self.class_body(node, out),
            "import_statement" | "import_from_statement" => self.import(node, out),
            "function_definition"
            | "global_statement"
            | "nonlocal_statement"
            | "pass_statement"
            | "break_statement"
            | "continue_statement"
            | "comment"
            | "future_import_statement" => {}
            _ => self.expression(scope, node, out),
        }
    }

    fn if_statement(&mut self, scope: ScopeId, node: Node<'_>, out: &mut Vec<Step>) {
        let Some(condition) = field(node, "condition") else {
            return;
        };
        if self.is_main_guard(scope, condition) {
            self.entries.push(Entry {
                unit: self.unit.clone(),
                label: format!("python -m {}", self.index.module_of(scope)),
                kind: "main".into(),
                chain: Vec::new(),
            });
        }
        self.expression(scope, condition, out);
        let yes =
            field(node, "consequence").map_or_else(Vec::new, |block| self.block(scope, block));
        let alternatives: Vec<_> = node
            .children_by_field_name("alternative", &mut node.walk())
            .collect();
        let no = self.alternatives(scope, &alternatives);
        out.push(branch(collapsed(condition, self.source), yes, no));
    }

    /// `elif` nests a branch in the "no" arm of the one before it.
    fn alternatives(&mut self, scope: ScopeId, alternatives: &[Node<'_>]) -> Vec<Step> {
        let Some((first, rest)) = alternatives.split_first() else {
            return Vec::new();
        };
        if first.kind() == "else_clause" {
            return field(*first, "body").map_or_else(Vec::new, |body| self.block(scope, body));
        }
        let mut steps = Vec::new();
        let Some(condition) = field(*first, "condition") else {
            return steps;
        };
        self.expression(scope, condition, &mut steps);
        let yes =
            field(*first, "consequence").map_or_else(Vec::new, |block| self.block(scope, block));
        let no = self.alternatives(scope, rest);
        steps.push(branch(collapsed(condition, self.source), yes, no));
        steps
    }

    fn match_statement(&mut self, scope: ScopeId, node: Node<'_>, out: &mut Vec<Step>) {
        let subject = field(node, "subject");
        if let Some(subject) = subject {
            self.expression(scope, subject, out);
        }
        let Some(body) = field(node, "body") else {
            return;
        };
        let mut arms = Vec::new();
        let mut exhaustive = false;
        for case in body.children_by_field_name("alternative", &mut body.walk()) {
            let patterns: Vec<_> = named_children(case)
                .into_iter()
                .filter(|child| child.kind() == "case_pattern")
                .collect();
            let guard = field(case, "guard");
            let mut label = format!(
                "case {}",
                patterns
                    .iter()
                    .map(|pattern| collapsed(*pattern, self.source))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let mut steps = Vec::new();
            if let Some(guard) = guard {
                label = format!("{label} {}", collapsed(guard, self.source));
                self.expression(scope, guard, &mut steps);
            } else if let [pattern] = patterns.as_slice() {
                exhaustive |= self.is_irrefutable(*pattern);
            }
            if let Some(consequence) = field(case, "consequence") {
                steps.extend(self.block(scope, consequence));
            }
            arms.push(Arm { label, body: steps });
        }
        if !exhaustive {
            arms.push(Arm {
                label: "no match".into(),
                body: Vec::new(),
            });
        }
        if arms.len() < 2 {
            out.extend(arms.into_iter().flat_map(|arm| arm.body));
            return;
        }
        let guard = subject.map(|subject| format!("match {}", collapsed(subject, self.source)));
        out.push(Step::Branch { guard, arms });
    }

    /// `case _:` and capture patterns (`case other:`) match anything.
    fn is_irrefutable(&self, pattern: Node<'_>) -> bool {
        let pattern_text = text(pattern, self.source);
        pattern_text == "_"
            || matches!(named_children(pattern).as_slice(),
                [only] if only.kind() == "dotted_name" && !pattern_text.contains('.'))
    }

    fn for_statement(&mut self, scope: ScopeId, node: Node<'_>, out: &mut Vec<Step>) {
        let (Some(left), Some(right)) = (field(node, "left"), field(node, "right")) else {
            return;
        };
        self.expression(scope, right, out);
        let body = field(node, "body").map_or_else(Vec::new, |body| self.block(scope, body));
        out.push(Step::Loop {
            label: Some(format!(
                "for {} in {}",
                collapsed(left, self.source),
                collapsed(right, self.source)
            )),
            body,
        });
        self.loop_else(scope, node, out);
    }

    /// The condition is evaluated before each iteration; it is lowered once, at the top
    /// of the loop body.
    fn while_statement(&mut self, scope: ScopeId, node: Node<'_>, out: &mut Vec<Step>) {
        let Some(condition) = field(node, "condition") else {
            return;
        };
        let mut body = Vec::new();
        self.expression(scope, condition, &mut body);
        if let Some(block) = field(node, "body") {
            body.extend(self.block(scope, block));
        }
        out.push(Step::Loop {
            label: Some(format!("while {}", collapsed(condition, self.source))),
            body,
        });
        self.loop_else(scope, node, out);
    }

    /// A loop's `else` runs when the loop ends without `break`, which the IR cannot
    /// distinguish; it is lowered as following the loop.
    fn loop_else(&mut self, scope: ScopeId, node: Node<'_>, out: &mut Vec<Step>) {
        if let Some(body) = field(node, "alternative").and_then(|clause| field(clause, "body")) {
            out.extend(self.block(scope, body));
        }
    }

    fn try_statement(&mut self, scope: ScopeId, node: Node<'_>, out: &mut Vec<Step>) {
        let clauses = named_children(node);
        let handlers: Vec<_> = clauses
            .iter()
            .filter(|clause| matches!(clause.kind(), "except_clause" | "except_group_clause"))
            .copied()
            .collect();
        let clause_block = |kind: &str| {
            clauses
                .iter()
                .find(|clause| clause.kind() == kind)
                .and_then(|clause| child_block(*clause))
        };
        let (else_block, finally_block) =
            (clause_block("else_clause"), clause_block("finally_clause"));

        let outer = self.protected;
        self.protected = outer || !handlers.is_empty();
        let mut body = field(node, "body").map_or_else(Vec::new, |body| self.block(scope, body));
        self.protected = outer;

        let trailing_exit = matches!(body.last(), Some(Step::Exit { .. }))
            .then(|| body.pop())
            .flatten();
        out.extend(body);
        if let Some(finally) = finally_block {
            out.extend(self.block(scope, finally));
        }
        let mut ok = else_block.map_or_else(Vec::new, |block| self.block(scope, block));
        ok.extend(trailing_exit);
        if handlers.is_empty() {
            out.extend(ok);
            return;
        }
        let mut arms = vec![Arm {
            label: "ok".into(),
            body: ok,
        }];
        for handler in handlers {
            let Some(block) = child_block(handler) else {
                continue;
            };
            let header = self
                .source
                .get(handler.start_byte()..block.start_byte())
                .unwrap_or_default();
            arms.push(Arm {
                label: header
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .trim_end_matches(':')
                    .to_owned(),
                body: self.block(scope, block),
            });
        }
        out.push(Step::Branch { guard: None, arms });
    }

    /// The label of a `raise`: the exception as constructed, message and all, so that
    /// each place that raises it reads apart; `re-raise` for a bare `raise`.
    fn raised(&self, node: Node<'_>) -> String {
        const MAX: usize = 80;
        let cause = field(node, "cause").map(|cause| cause.id());
        let exception = named_children(node)
            .into_iter()
            .find(|child| Some(child.id()) != cause && child.kind() != "comment");
        let Some(exception) = exception else {
            return "re-raise".into();
        };
        let raised = collapsed(exception, self.source);
        match raised.char_indices().nth(MAX) {
            Some((cut, _)) => format!("{}…", &raised[..cut]),
            None => raised,
        }
    }

    /// Class bodies run when the class statement does, in the enclosing unit.
    fn class_body(&mut self, def: Node<'_>, out: &mut Vec<Step>) {
        let scope = self.index.class_scopes.get(&(self.file, def.id())).copied();
        if let (Some(scope), Some(body)) = (scope, field(def, "body")) {
            out.extend(self.block(scope, body));
        }
    }

    fn import(&mut self, node: Node<'_>, out: &mut Vec<Step>) {
        let file = &self.index.parsed(self.file).file;
        let importer = Importer {
            module: &file.module,
            is_package: file.is_package,
        };
        for load in imports::parse(node, self.source, importer).loads {
            if self.index.modules.contains_key(&load.module) {
                let targets = vec![exact(module_body(&load.module))];
                let call = self.push(node, load.text, targets, Mode::Call, None, out);
                self.calls[call].loads = Some(load.module);
            }
        }
    }

    fn decorated(&mut self, scope: ScopeId, node: Node<'_>, out: &mut Vec<Step>) {
        let Some(definition) = field(node, "definition") else {
            return;
        };
        let unit = self
            .index
            .definitions
            .get(&(self.file, definition.id()))
            .cloned();
        let decorators: Vec<_> = named_children(node)
            .into_iter()
            .filter(|child| child.kind() == "decorator")
            .filter_map(|decorator| decorator.named_child(0))
            .collect();
        for (position, &decorator) in decorators.iter().enumerate() {
            let (callee, arguments) = if decorator.kind() == "call" {
                (field(decorator, "function"), field(decorator, "arguments"))
            } else {
                (Some(decorator), None)
            };
            let Some(callee) = callee else {
                continue;
            };
            let text = compact(callee, self.source);
            if arguments.is_none() && is_marker(&text) {
                continue;
            }
            if let Some(object) = field(callee, "object") {
                self.expression(scope, object, out);
            }
            if let Some(arguments) = arguments {
                self.expression(scope, arguments, out);
            }
            if let (Some(unit), Some(via)) = (&unit, self.receiver_channel(scope, callee)) {
                let targets = vec![exact(unit.clone())];
                self.push(decorator, text, targets, Mode::Register, Some(via), out);
            } else {
                let targets = self.resolver.callee(scope, callee);
                self.push(decorator, text, targets, Mode::Call, None, out);
            }
            if let (Some(unit), Some(arguments)) = (&unit, arguments) {
                let wrappers = &decorators[position + 1..];
                self.decorator_entry(scope, callee, arguments, unit, wrappers);
            }
        }
        if definition.kind() == "class_definition" {
            self.class_body(definition, out);
        }
    }

    /// `@router.post("/path")` declares an `http` entry; `@app.on_event("startup")` a
    /// `lifecycle` one. Decorators below it wrap the function, innermost last.
    fn decorator_entry(
        &mut self,
        scope: ScopeId,
        callee: Node<'_>,
        arguments: Node<'_>,
        unit: &UnitId,
        wrappers: &[Node<'_>],
    ) {
        let Some(method) = last_name(callee, self.source) else {
            return;
        };
        let Some(literal) = first_string_argument(arguments, self.source) else {
            return;
        };
        let (kind, label) = if HTTP_METHODS.contains(&method) {
            ("http", format!("{} {literal}", method.to_uppercase()))
        } else if method == "on_event" {
            ("lifecycle", literal)
        } else {
            return;
        };
        let chain = wrappers
            .iter()
            .filter_map(|&decorator| {
                let (callee, depth) = match field(decorator, "function") {
                    Some(callee) if decorator.kind() == "call" => (callee, 2),
                    _ => (decorator, 1),
                };
                let target = self.resolver.callee(scope, callee).into_iter().next()?;
                self.resolver.wrapper(&target.unit, depth)
            })
            .collect();
        self.entries.push(Entry {
            unit: unit.clone(),
            label,
            kind: kind.into(),
            chain,
        });
    }

    fn expression(&mut self, scope: ScopeId, node: Node<'_>, out: &mut Vec<Step>) {
        match node.kind() {
            "call" => self.call(scope, node, out),
            "lambda" | "function_definition" | "class_definition" => {}
            "conditional_expression" => self.conditional(scope, node, out),
            "list_comprehension"
            | "set_comprehension"
            | "dictionary_comprehension"
            | "generator_expression" => self.comprehension(scope, node, out),
            _ => {
                for child in named_children(node) {
                    self.expression(scope, child, out);
                }
            }
        }
    }

    /// `a if condition else b`.
    fn conditional(&mut self, scope: ScopeId, node: Node<'_>, out: &mut Vec<Step>) {
        let [yes, condition, no] = named_children(node)[..] else {
            return;
        };
        self.expression(scope, condition, out);
        let (mut yes_steps, mut no_steps) = (Vec::new(), Vec::new());
        self.expression(scope, yes, &mut yes_steps);
        self.expression(scope, no, &mut no_steps);
        if !(yes_steps.is_empty() && no_steps.is_empty()) {
            out.push(branch(
                collapsed(condition, self.source),
                yes_steps,
                no_steps,
            ));
        }
    }

    /// The first iterable is evaluated once; every other clause and the element run
    /// per item. Generators are lowered as if consumed where they are created.
    fn comprehension(&mut self, scope: ScopeId, node: Node<'_>, out: &mut Vec<Step>) {
        let clauses: Vec<_> = named_children(node)
            .into_iter()
            .filter(|child| matches!(child.kind(), "for_in_clause" | "if_clause"))
            .collect();
        let Some((first, rest)) = clauses.split_first() else {
            return;
        };
        let (left, right) = (field(*first, "left"), field(*first, "right"));
        if let Some(right) = right {
            self.expression(scope, right, out);
        }
        let mut body = Vec::new();
        for clause in rest {
            let evaluated = if clause.kind() == "for_in_clause" {
                field(*clause, "right")
            } else {
                Some(*clause)
            };
            if let Some(evaluated) = evaluated {
                self.expression(scope, evaluated, &mut body);
            }
        }
        if let Some(element) = field(node, "body") {
            self.expression(scope, element, &mut body);
        }
        if body.is_empty() {
            return;
        }
        let label = match (left, right) {
            (Some(left), Some(right)) => Some(format!(
                "for {} in {}",
                collapsed(left, self.source),
                collapsed(right, self.source)
            )),
            _ => None,
        };
        out.push(Step::Loop { label, body });
    }

    fn call(&mut self, scope: ScopeId, node: Node<'_>, out: &mut Vec<Step>) {
        let Some(callee) = field(node, "function") else {
            return;
        };
        match callee.kind() {
            "identifier" => {}
            "attribute" => {
                // `super()` only selects where method lookup starts.
                if let Some(object) = field(callee, "object")
                    && !matches!(self.resolver.eval(scope, object), Value::Super(_))
                {
                    self.expression(scope, object, out);
                }
            }
            _ => self.expression(scope, callee, out),
        }
        let text = compact(callee, self.source);
        let arguments = field(node, "arguments");
        if last_name(callee, self.source).is_some_and(|name| SPAWNERS.contains(&name)) {
            self.spawn(scope, node, text, arguments, out);
            return;
        }

        let mut handed = Vec::new();
        for argument in arguments.map(argument_values).unwrap_or_default() {
            match self.handoff(scope, argument) {
                Some(target) => handed.push((argument, target)),
                None => self.expression(scope, argument, out),
            }
        }
        let targets = self.resolver.callee(scope, callee);
        if handed.is_empty() {
            self.push(node, text, targets, Mode::Call, None, out);
            return;
        }
        self.lifecycle_entries(callee, arguments, &handed);
        if let Some(receiver) = targets.first().map(|target| target.unit.clone()) {
            // A known callee runs now and is handed each function to invoke later.
            self.push(node, text, targets, Mode::Call, None, out);
            for (argument, target) in handed {
                let argument_text = handed_text(argument, self.source);
                let targets = vec![exact(target)];
                self.push(
                    node,
                    argument_text,
                    targets,
                    Mode::Register,
                    Some(receiver.clone()),
                    out,
                );
            }
        } else {
            let via = self.receiver_channel(scope, callee);
            let targets = handed
                .into_iter()
                .map(|(_, target)| exact(target))
                .collect();
            self.push(node, text, targets, Mode::Register, via, out);
        }
    }

    /// `asyncio.create_task(work(x))`: `work` starts concurrently. The coroutine call
    /// itself does not run `work`, so it is folded into the spawn.
    fn spawn(
        &mut self,
        scope: ScopeId,
        node: Node<'_>,
        text: String,
        arguments: Option<Node<'_>>,
        out: &mut Vec<Step>,
    ) {
        let values = arguments.map(argument_values).unwrap_or_default();
        let mut targets = Vec::new();
        for (position, value) in values.into_iter().enumerate() {
            match field(value, "function") {
                Some(coroutine) if position == 0 && value.kind() == "call" => {
                    if let Some(object) = field(coroutine, "object") {
                        self.expression(scope, object, out);
                    }
                    if let Some(arguments) = field(value, "arguments") {
                        self.expression(scope, arguments, out);
                    }
                    targets = self.resolver.callee(scope, coroutine);
                }
                _ => self.expression(scope, value, out),
            }
        }
        self.push(node, text, targets, Mode::Spawn, None, out);
    }

    /// `app.add_event_handler("startup", init_db)` declares a `lifecycle` entry.
    fn lifecycle_entries(
        &mut self,
        callee: Node<'_>,
        arguments: Option<Node<'_>>,
        handed: &[(Node<'_>, UnitId)],
    ) {
        if last_name(callee, self.source) != Some("add_event_handler") {
            return;
        }
        let Some(event) = arguments.and_then(|a| first_string_argument(a, self.source)) else {
            return;
        };
        for (_, target) in handed {
            self.entries.push(Entry {
                unit: target.clone(),
                label: event.clone(),
                kind: "lifecycle".into(),
                chain: Vec::new(),
            });
        }
    }

    /// The unit an argument hands over to be invoked later: a lambda, a function, or a
    /// module-level variable that is itself a channel.
    fn handoff(&self, scope: ScopeId, argument: Node<'_>) -> Option<UnitId> {
        match argument.kind() {
            "lambda" => self
                .index
                .definitions
                .get(&(self.file, argument.id()))
                .cloned(),
            "identifier" | "attribute" => match self.resolver.eval(scope, argument) {
                Value::Function(unit) => Some(unit),
                Value::Variable(channel) if self.channels.contains(&channel) => Some(channel),
                _ => None,
            },
            _ => None,
        }
    }

    /// `router` in `router.post(…)`, when it is a module-level variable.
    fn receiver_channel(&mut self, scope: ScopeId, callee: Node<'_>) -> Option<UnitId> {
        let object = field(callee, "object")?;
        let Value::Variable(channel) = self.resolver.eval(scope, object) else {
            return None;
        };
        self.vias.insert(channel.clone());
        Some(channel)
    }

    fn is_main_guard(&self, scope: ScopeId, condition: Node<'_>) -> bool {
        let is_module = self.index.scopes[scope].parent.is_none();
        let normalized = compact(condition, self.source).replace('\'', "\"");
        is_module
            && matches!(
                normalized.as_str(),
                "__name__==\"__main__\"" | "\"__main__\"==__name__"
            )
    }

    /// Records a call site and appends a step for it. Returns its provisional index.
    fn push(
        &mut self,
        node: Node<'_>,
        text: String,
        targets: Vec<Target>,
        mode: Mode,
        via: Option<UnitId>,
        out: &mut Vec<Step>,
    ) -> usize {
        let index = self.calls.len();
        self.calls.push(RawCall {
            text,
            start: node.start_byte(),
            targets,
            propagates: mode == Mode::Call && !self.protected,
            mode,
            via,
            location: location(node, &self.index.parsed(self.file).file.uri),
            loads: None,
        });
        out.push(Step::Call {
            call: CallKey::new(index.to_string()),
        });
        index
    }
}

impl Lowered {
    /// Drops the calls `keep` rejects, assigns keys in source order, and simplifies the
    /// body. The body is omitted when it says no more than the call list.
    pub(crate) fn finish(
        self,
        keep: impl Fn(&RawCall) -> bool,
    ) -> (Vec<Invocation>, Option<Vec<Step>>) {
        let mut order: Vec<usize> = (0..self.calls.len())
            .filter(|&index| keep(&self.calls[index]))
            .collect();
        order.sort_by_key(|&index| (self.calls[index].start, index));

        let mut occurrences: HashMap<&str, usize> = HashMap::new();
        let mut keys = HashMap::new();
        let mut invocations = Vec::new();
        for index in order {
            let raw = &self.calls[index];
            let occurrence = occurrences.entry(raw.text.as_str()).or_default();
            let mut invocation =
                Invocation::new(raw.text.clone(), *occurrence, raw.targets.clone());
            *occurrence += 1;
            invocation.mode = raw.mode;
            invocation.via.clone_from(&raw.via);
            invocation.propagates = raw.propagates;
            invocation.location = Some(raw.location.clone());
            keys.insert(index.to_string(), invocation.key.clone());
            invocations.push(invocation);
        }

        let mut steps = rekey(self.steps, &keys);
        if matches!(
            steps.last(),
            Some(Step::Exit {
                kind: ExitKind::Return,
                label: None
            })
        ) {
            steps.pop();
        }
        let linear = steps.len() == invocations.len()
            && steps.iter().zip(&invocations).all(
                |(step, invocation)| matches!(step, Step::Call { call } if *call == invocation.key),
            );
        let body = (!steps.is_empty() && !linear).then_some(steps);
        (invocations, body)
    }
}

/// Replaces provisional call indices with keys, dropping steps for removed calls and
/// any branch or loop left with nothing in it.
fn rekey(steps: Vec<Step>, keys: &HashMap<String, CallKey>) -> Vec<Step> {
    steps
        .into_iter()
        .filter_map(|step| match step {
            Step::Call { call } => keys
                .get(call.as_str())
                .map(|key| Step::Call { call: key.clone() }),
            Step::Branch { guard, arms } => {
                let arms: Vec<_> = arms
                    .into_iter()
                    .map(|arm| Arm {
                        label: arm.label,
                        body: rekey(arm.body, keys),
                    })
                    .collect();
                arms.iter()
                    .any(|arm| !arm.body.is_empty())
                    .then_some(Step::Branch { guard, arms })
            }
            Step::Loop { label, body } => {
                let body = rekey(body, keys);
                (!body.is_empty()).then_some(Step::Loop { label, body })
            }
            Step::Dag { nodes } => Some(Step::Dag { nodes }),
            other => Some(other),
        })
        .collect()
}

fn branch(guard: String, yes: Vec<Step>, no: Vec<Step>) -> Step {
    Step::Branch {
        guard: Some(guard),
        arms: vec![
            Arm {
                label: "yes".into(),
                body: yes,
            },
            Arm {
                label: "no".into(),
                body: no,
            },
        ],
    }
}

fn exact(unit: UnitId) -> Target {
    Target {
        unit,
        certainty: Certainty::Exact,
    }
}

fn is_marker(callee: &str) -> bool {
    MARKERS.contains(&callee)
        || [".setter", ".getter", ".deleter"]
            .iter()
            .any(|suffix| callee.ends_with(suffix))
}

fn child_block(node: Node<'_>) -> Option<Node<'_>> {
    named_children(node)
        .into_iter()
        .find(|child| child.kind() == "block")
}

/// The expressions passed to a call: positional values and keyword values, in order. A
/// bare generator argument (`any(x for x in xs)`) is the single value.
fn argument_values(arguments: Node<'_>) -> Vec<Node<'_>> {
    if arguments.kind() != "argument_list" {
        return vec![arguments];
    }
    named_children(arguments)
        .into_iter()
        .filter_map(|argument| match argument.kind() {
            "keyword_argument" => field(argument, "value"),
            "comment" => None,
            _ => Some(argument),
        })
        .collect()
}

fn first_string_argument(arguments: Node<'_>, source: &str) -> Option<String> {
    let first = named_children(arguments).into_iter().next()?;
    string_literal(first, source)
}

/// How a handed-over function is written: its name, or `lambda`.
fn handed_text(argument: Node<'_>, source: &str) -> String {
    if argument.kind() == "lambda" {
        "lambda".into()
    } else {
        compact(argument, source)
    }
}
