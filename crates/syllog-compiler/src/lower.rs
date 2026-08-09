//! AST-to-HIR lowering after successful semantic analysis.

use std::collections::BTreeMap;

use syllog_parser::{
    Ast, Diagnostic, Expr, ExprKind, Item, Literal, Pattern, PatternKind, Severity, Span,
    StatementKind, TypeKind, TypeNode,
};
use syllog_semantic::{
    Analysis, PrimitiveType, ResolvedType, SymbolTable, TypeSymbolKind, analyze,
};

use crate::hir::{
    DefId, HirBlock, HirDefinition, HirDefinitionKind, HirExprKind, HirFunction, HirMatchArm,
    HirMember, HirModule, HirParameter, HirPattern, HirPipeline, HirProgram, HirStatement,
    HirVariant, ModuleId, TypedExpr,
};

const SOURCE_MODULE: ModuleId = ModuleId(0);
const BUILTIN_MODULE: ModuleId = ModuleId(u32::MAX);

/// Lowers a semantically valid AST into fully typed, identity-resolved HIR.
///
/// The bootstrap API accepts the symbol table to make the phase boundary
/// explicit. Expression types are recomputed by the semantic analyzer until
/// the incremental database introduced in the next milestone owns both query
/// results.
///
/// # Errors
///
/// Returns semantic diagnostics or HIR invariant diagnostics. No executable
/// HIR is returned when any unresolved type or name remains.
pub fn lower_to_hir(ast: &Ast, symbols: &SymbolTable) -> Result<HirProgram, Vec<Diagnostic>> {
    let analysis = analyze("<hir>", ast);
    if !analysis.diagnostics.is_empty() {
        return Err(analysis.diagnostics);
    }
    if analysis.symbols != *symbols {
        return Err(vec![hir_error(
            ast.span,
            "symbol table does not belong to the supplied syntax tree",
        )]);
    }
    Lowerer::new(ast, &analysis).lower()
}

pub(crate) fn lower_module_to_hir(
    ast: &Ast,
    analysis: &Analysis,
    module: ModuleId,
    globals: BTreeMap<String, DefId>,
    members: BTreeMap<String, DefId>,
    next_definition: u32,
) -> Result<(HirModule, Option<DefId>, u32), Vec<Diagnostic>> {
    Lowerer::new_planned(ast, analysis, module, globals, members, next_definition).lower_module()
}

struct Lowerer<'a> {
    ast: &'a Ast,
    analysis: &'a Analysis,
    next_definition: u32,
    globals: BTreeMap<String, DefId>,
    members: BTreeMap<String, DefId>,
    expression_types: BTreeMap<Span, ResolvedType>,
    diagnostics: Vec<Diagnostic>,
    module: ModuleId,
}

impl<'a> Lowerer<'a> {
    fn new(ast: &'a Ast, analysis: &'a Analysis) -> Self {
        let expression_types = analysis
            .expression_types
            .iter()
            .map(|expression| (expression.span, expression.ty.clone()))
            .collect();
        Self {
            ast,
            analysis,
            next_definition: 0,
            globals: BTreeMap::new(),
            members: BTreeMap::new(),
            expression_types,
            diagnostics: Vec::new(),
            module: SOURCE_MODULE,
        }
    }

    fn new_planned(
        ast: &'a Ast,
        analysis: &'a Analysis,
        module: ModuleId,
        globals: BTreeMap<String, DefId>,
        members: BTreeMap<String, DefId>,
        next_definition: u32,
    ) -> Self {
        let expression_types = analysis
            .expression_types
            .iter()
            .map(|expression| (expression.span, expression.ty.clone()))
            .collect();
        Self {
            ast,
            analysis,
            next_definition,
            globals,
            members,
            expression_types,
            diagnostics: Vec::new(),
            module,
        }
    }

    fn lower(self) -> Result<HirProgram, Vec<Diagnostic>> {
        let (module, entry, _) = self.lower_module()?;
        Ok(HirProgram {
            schema_version: 1,
            modules: vec![module],
            entry,
        })
    }

    fn lower_module(mut self) -> Result<(HirModule, Option<DefId>, u32), Vec<Diagnostic>> {
        self.index_globals();
        self.index_members();
        let definitions = self
            .ast
            .items
            .iter()
            .map(|item| self.lower_definition(item))
            .collect();
        if !self.diagnostics.is_empty() {
            return Err(self.diagnostics);
        }
        let entry = self
            .ast
            .items
            .iter()
            .any(|item| item_name(item) == "main")
            .then(|| self.globals["main"]);
        Ok((
            HirModule {
                id: self.module,
                definitions,
            },
            entry,
            self.next_definition,
        ))
    }

    fn index_globals(&mut self) {
        for item in &self.ast.items {
            let name = item_name(item);
            if !self.globals.contains_key(name) {
                let id = self.allocate();
                self.globals.insert(name.to_owned(), id);
            }
        }
    }

    fn index_members(&mut self) {
        for item in &self.ast.items {
            match item {
                Item::Struct(node) => {
                    for field in &node.fields {
                        let key = format!("{}.{}", node.name, field.name);
                        if !self.members.contains_key(&key) {
                            let id = self.allocate();
                            self.members.insert(key, id);
                        }
                    }
                }
                Item::Enum(node) => {
                    for variant in &node.variants {
                        let key = format!("{}::{}", node.name, variant.name);
                        if !self.members.contains_key(&key) {
                            let id = self.allocate();
                            self.members.insert(key, id);
                        }
                    }
                }
                Item::State(node) => {
                    for field in &node.fields {
                        let key = format!("{}.{}", node.name, field.name);
                        if !self.members.contains_key(&key) {
                            let id = self.allocate();
                            self.members.insert(key, id);
                        }
                    }
                }
                Item::Function(_) | Item::Agent(_) | Item::Pipeline(_) | Item::SafetyBound(_) => {}
            }
        }
    }

    fn lower_definition(&mut self, item: &Item) -> HirDefinition {
        let name = item_name(item).to_owned();
        let id = self.globals[&name];
        let (kind, span) = match item {
            Item::Struct(node) => {
                let fields = node
                    .fields
                    .iter()
                    .map(|field| HirMember {
                        id: self.members[&format!("{}.{}", node.name, field.name)],
                        name: field.name.clone(),
                        ty: self.lower_type(&field.ty),
                        span: field.span,
                    })
                    .collect();
                (HirDefinitionKind::Struct { fields }, node.span)
            }
            Item::Enum(node) => {
                let variants = node
                    .variants
                    .iter()
                    .map(|variant| HirVariant {
                        id: self.members[&format!("{}::{}", node.name, variant.name)],
                        name: variant.name.clone(),
                        fields: variant
                            .fields
                            .iter()
                            .map(|field| self.lower_type(field))
                            .collect(),
                        span: variant.span,
                    })
                    .collect();
                (HirDefinitionKind::Enum { variants }, node.span)
            }
            Item::Function(node) => {
                let mut scope = BTreeMap::new();
                let parameters = self.lower_parameters(&node.parameters, &mut scope);
                let result = node
                    .return_type
                    .as_ref()
                    .map_or(ResolvedType::Unit, |ty| self.lower_type(ty));
                let body = self.lower_block(&node.body, &mut scope);
                (
                    HirDefinitionKind::Function(HirFunction {
                        is_test: has_attribute(node, "test"),
                        asynchronous: node.asynchronous,
                        parameters,
                        result,
                        body,
                    }),
                    node.span,
                )
            }
            Item::State(node) => {
                let fields = node
                    .fields
                    .iter()
                    .map(|field| HirMember {
                        id: self.members[&format!("{}.{}", node.name, field.name)],
                        name: field.name.clone(),
                        ty: self.lower_type(&field.ty),
                        span: field.span,
                    })
                    .collect();
                (HirDefinitionKind::State { fields }, node.span)
            }
            Item::Agent(node) => (HirDefinitionKind::Agent, node.span),
            Item::Pipeline(node) => {
                let mut scope = BTreeMap::new();
                let parameters = self.lower_parameters(&node.parameters, &mut scope);
                let result = node
                    .return_type
                    .as_ref()
                    .map_or(ResolvedType::Unit, |ty| self.lower_type(ty));
                let agent = node
                    .property("agent")
                    .and_then(|property| expression_name(&property.value))
                    .and_then(|name| self.globals.get(name).copied());
                let body = node
                    .property("result")
                    .map(|property| self.lower_expression(&property.value, &mut scope));
                (
                    HirDefinitionKind::Pipeline(HirPipeline {
                        parameters,
                        result,
                        agent,
                        body,
                    }),
                    node.span,
                )
            }
            Item::SafetyBound(node) => (HirDefinitionKind::SafetyBound, node.span),
        };
        HirDefinition {
            id,
            name,
            kind,
            span,
        }
    }

    fn lower_parameters(
        &mut self,
        parameters: &[syllog_parser::Parameter],
        scope: &mut BTreeMap<String, DefId>,
    ) -> Vec<HirParameter> {
        parameters
            .iter()
            .map(|parameter| {
                let id = self.allocate();
                scope.insert(parameter.name.clone(), id);
                HirParameter {
                    id,
                    name: parameter.name.clone(),
                    ty: self.lower_type(&parameter.ty),
                    span: parameter.span,
                }
            })
            .collect()
    }

    fn lower_block(
        &mut self,
        block: &syllog_parser::Block,
        scope: &mut BTreeMap<String, DefId>,
    ) -> HirBlock {
        let statements = block
            .statements
            .iter()
            .map(|statement| match &statement.kind {
                StatementKind::Let { name, ty, value } => {
                    let value = self.lower_expression(value, scope);
                    let ty = ty
                        .as_ref()
                        .map_or_else(|| value.ty.clone(), |ty| self.lower_type(ty));
                    let definition = self.allocate();
                    scope.insert(name.clone(), definition);
                    HirStatement::Let {
                        definition,
                        name: name.clone(),
                        ty,
                        value,
                    }
                }
                StatementKind::Return(value) => HirStatement::Return(
                    value
                        .as_ref()
                        .map(|value| self.lower_expression(value, scope)),
                ),
                StatementKind::Expression(expression) => {
                    HirStatement::Expression(self.lower_expression(expression, scope))
                }
            })
            .collect();
        HirBlock {
            statements,
            span: block.span,
        }
    }

    fn lower_expression(
        &mut self,
        expression: &Expr,
        scope: &mut BTreeMap<String, DefId>,
    ) -> TypedExpr {
        let ty = self
            .expression_types
            .get(&expression.span)
            .cloned()
            .unwrap_or_else(|| Self::fallback_expression_type(expression, scope));
        let kind = match &expression.kind {
            ExprKind::Await(operand) => {
                HirExprKind::Await(Box::new(self.lower_expression(operand, scope)))
            }
            ExprKind::Literal(Literal::Identifier(name)) if name != "none" => self
                .resolve_value(name, scope, expression.span)
                .map_or_else(
                    || HirExprKind::Literal(Literal::Identifier(name.clone())),
                    |definition| HirExprKind::Reference { definition },
                ),
            ExprKind::Literal(literal) => HirExprKind::Literal(literal.clone()),
            ExprKind::Path(segments) => {
                let name = segments.join("::");
                let definition = self.resolve_member(&name, expression.span);
                HirExprKind::Reference { definition }
            }
            ExprKind::Array(items) => HirExprKind::Array(
                items
                    .iter()
                    .map(|item| self.lower_expression(item, scope))
                    .collect(),
            ),
            ExprKind::Call { callee, arguments } => HirExprKind::Call {
                callee: Box::new(self.lower_expression(callee, scope)),
                arguments: arguments
                    .iter()
                    .map(|argument| self.lower_expression(&argument.value, scope))
                    .collect(),
            },
            ExprKind::Field { base, name } => {
                let base = self.lower_expression(base, scope);
                let owner = named_type(&base.ty).unwrap_or("<error>");
                let field = self.resolve_member(&format!("{owner}.{name}"), expression.span);
                HirExprKind::Field {
                    base: Box::new(base),
                    field,
                }
            }
            ExprKind::Unary { operator, operand } => HirExprKind::Unary {
                operator: *operator,
                operand: Box::new(self.lower_expression(operand, scope)),
            },
            ExprKind::Binary {
                operator,
                left,
                right,
            } => HirExprKind::Binary {
                operator: *operator,
                left: Box::new(self.lower_expression(left, scope)),
                right: Box::new(self.lower_expression(right, scope)),
            },
            ExprKind::Match { value, arms } => {
                let value = Box::new(self.lower_expression(value, scope));
                let arms = arms
                    .iter()
                    .map(|arm| {
                        let mut arm_scope = scope.clone();
                        let pattern = self.lower_pattern(&arm.pattern, &mut arm_scope);
                        let guard = arm
                            .guard
                            .as_ref()
                            .map(|guard| self.lower_expression(guard, &mut arm_scope));
                        let body = self.lower_expression(&arm.body, &mut arm_scope);
                        HirMatchArm {
                            pattern,
                            guard,
                            body,
                            span: arm.span,
                        }
                    })
                    .collect();
                HirExprKind::Match { value, arms }
            }
            ExprKind::Block(block) => {
                let mut nested = scope.clone();
                HirExprKind::Block(self.lower_block(block, &mut nested))
            }
        };
        TypedExpr {
            kind,
            ty,
            span: expression.span,
        }
    }

    fn lower_pattern(
        &mut self,
        pattern: &Pattern,
        scope: &mut BTreeMap<String, DefId>,
    ) -> HirPattern {
        match &pattern.kind {
            PatternKind::Wildcard => HirPattern::Wildcard,
            PatternKind::Literal(literal) => HirPattern::Literal(literal.clone()),
            PatternKind::Path(path) if path.len() == 1 => {
                let definition = self.allocate();
                scope.insert(path[0].clone(), definition);
                HirPattern::Binding { definition }
            }
            PatternKind::Path(path) => HirPattern::Variant {
                definition: self.resolve_member(&path.join("::"), pattern.span),
                fields: Vec::new(),
            },
            PatternKind::Constructor { path, fields } => HirPattern::Variant {
                definition: self.resolve_member(&path.join("::"), pattern.span),
                fields: fields
                    .iter()
                    .map(|field| self.lower_pattern(field, scope))
                    .collect(),
            },
        }
    }

    fn lower_type(&self, node: &TypeNode) -> ResolvedType {
        match &node.kind {
            TypeKind::Array(inner) => ResolvedType::Array(Box::new(self.lower_type(inner))),
            TypeKind::Tuple(items) if items.is_empty() => ResolvedType::Unit,
            TypeKind::Tuple(items) => {
                ResolvedType::Tuple(items.iter().map(|item| self.lower_type(item)).collect())
            }
            TypeKind::Path {
                segments,
                arguments,
            } if segments.len() == 1 && segments[0] == "Option" && arguments.len() == 1 => {
                ResolvedType::Option(Box::new(self.lower_type(&arguments[0])))
            }
            TypeKind::Path {
                segments,
                arguments,
            } if segments.len() == 1 && segments[0] == "Result" && arguments.len() == 2 => {
                ResolvedType::Result(
                    Box::new(self.lower_type(&arguments[0])),
                    Box::new(self.lower_type(&arguments[1])),
                )
            }
            TypeKind::Path { segments, .. } if segments.len() == 1 => {
                let name = &segments[0];
                self.analysis
                    .symbols
                    .types
                    .get(name)
                    .map_or(ResolvedType::Error, |symbol| match symbol.kind {
                        TypeSymbolKind::Primitive => primitive(name),
                        TypeSymbolKind::Struct => ResolvedType::Struct(name.clone()),
                        TypeSymbolKind::Enum => ResolvedType::Enum(name.clone()),
                        TypeSymbolKind::State => ResolvedType::State(name.clone()),
                        TypeSymbolKind::Generic { .. } => ResolvedType::Error,
                    })
            }
            TypeKind::Path { .. } => ResolvedType::Error,
        }
    }

    fn fallback_expression_type(
        expression: &Expr,
        scope: &BTreeMap<String, DefId>,
    ) -> ResolvedType {
        match &expression.kind {
            ExprKind::Await(operand) => Self::fallback_expression_type(operand, scope),
            ExprKind::Literal(Literal::Identifier(name)) if scope.contains_key(name) => {
                ResolvedType::Unknown
            }
            ExprKind::Path(segments) if segments.len() == 2 => {
                ResolvedType::Enum(segments[0].clone())
            }
            _ => ResolvedType::Error,
        }
    }

    fn resolve_value(
        &mut self,
        name: &str,
        scope: &BTreeMap<String, DefId>,
        span: Span,
    ) -> Option<DefId> {
        scope
            .get(name)
            .or_else(|| self.globals.get(name))
            .copied()
            .or_else(|| {
                self.diagnostics
                    .push(hir_error(span, format!("unresolved HIR value '{name}'")));
                None
            })
    }

    fn resolve_member(&mut self, name: &str, span: Span) -> DefId {
        self.members.get(name).copied().unwrap_or_else(|| {
            builtin_definition(name).unwrap_or_else(|| {
                self.diagnostics
                    .push(hir_error(span, format!("unresolved HIR member '{name}'")));
                DefId {
                    module: BUILTIN_MODULE,
                    index: u32::MAX,
                }
            })
        })
    }

    fn allocate(&mut self) -> DefId {
        let id = DefId {
            module: self.module,
            index: self.next_definition,
        };
        self.next_definition += 1;
        id
    }
}

fn has_attribute(function: &syllog_parser::FunctionNode, name: &str) -> bool {
    function
        .attributes
        .iter()
        .any(|attribute| attribute.name == name)
}

fn item_name(item: &Item) -> &str {
    match item {
        Item::Struct(node) => &node.name,
        Item::Enum(node) => &node.name,
        Item::Function(node) => &node.name,
        Item::State(node) => &node.name,
        Item::Agent(node) => &node.name,
        Item::Pipeline(node) => &node.name,
        Item::SafetyBound(node) => &node.name,
    }
}

fn expression_name(expression: &Expr) -> Option<&str> {
    match &expression.kind {
        ExprKind::Literal(Literal::Identifier(name)) => Some(name),
        ExprKind::Path(path) => path.last().map(String::as_str),
        _ => None,
    }
}

fn named_type(ty: &ResolvedType) -> Option<&str> {
    match ty {
        ResolvedType::Struct(name) | ResolvedType::State(name) => Some(name),
        _ => None,
    }
}

fn primitive(name: &str) -> ResolvedType {
    let primitive = match name {
        "Bool" => PrimitiveType::Bool,
        "String" => PrimitiveType::String,
        "Str" => PrimitiveType::Str,
        "Char" => PrimitiveType::Char,
        "Bytes" => PrimitiveType::Bytes,
        "Duration" => PrimitiveType::Duration,
        "Size" => PrimitiveType::Size,
        "AgentRef" => PrimitiveType::AgentRef,
        "Provider" => PrimitiveType::Provider,
        "Text" => PrimitiveType::Text,
        integer if integer.starts_with('I') => PrimitiveType::Signed(integer.to_owned()),
        integer if integer.starts_with('U') => PrimitiveType::Unsigned(integer.to_owned()),
        float if float.starts_with('F') => PrimitiveType::Float(float.to_owned()),
        _ => return ResolvedType::Error,
    };
    ResolvedType::Primitive(primitive)
}

fn builtin_definition(name: &str) -> Option<DefId> {
    let index = match name {
        "Option::some" => 0,
        "Option::none" => 1,
        "Result::ok" => 2,
        "Result::err" => 3,
        _ => return None,
    };
    Some(DefId {
        module: BUILTIN_MODULE,
        index,
    })
}

fn hir_error(span: Span, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        code: "SYL3001".into(),
        severity: Severity::Error,
        message: message.into(),
        file: "<hir>".into(),
        span,
    }
}
