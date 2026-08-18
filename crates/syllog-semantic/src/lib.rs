//! Static semantic analysis for Syllog.

mod modules;
mod ownership;
mod types;

pub use modules::*;
pub use ownership::*;
pub use types::*;

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use syllog_parser::{
    AgentNode, Ast, BinaryOperator, Block, CheckResult, Diagnostic, EnumNode, Expr, ExprKind,
    FunctionNode, Item, Literal, MatchArm, Pattern, PatternKind, PipelineNode, Property, Severity,
    Span, StateNode, StatementKind, StructNode, TypeKind, TypeNode,
};

/// Output of semantic analysis.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Analysis {
    /// Collected global symbols.
    pub symbols: SymbolTable,
    /// Source-positioned semantic errors.
    pub diagnostics: Vec<Diagnostic>,
    /// Resolved executable expression types in source order.
    pub expression_types: Vec<ExpressionType>,
}

#[derive(Debug, Clone)]
struct StructDefinition {
    fields: BTreeMap<String, ResolvedType>,
}

#[derive(Debug, Clone)]
struct EnumDefinition {
    variants: Vec<(String, Vec<ResolvedType>)>,
}

#[derive(Debug, Clone)]
struct FunctionSignature {
    parameters: Vec<ResolvedType>,
    result: ResolvedType,
}

#[derive(Debug, Clone, Default)]
struct AgentContract {
    input: Option<ResolvedType>,
    output: Option<ResolvedType>,
}

struct Analyzer<'a> {
    file: &'a str,
    ast: &'a Ast,
    symbols: SymbolTable,
    diagnostics: Vec<Diagnostic>,
    expression_types: BTreeMap<Span, ResolvedType>,
    structs: HashMap<String, StructDefinition>,
    enums: HashMap<String, EnumDefinition>,
    functions: HashMap<String, FunctionSignature>,
    agents: HashMap<String, AgentContract>,
    states: HashMap<String, Vec<ResolvedType>>,
    async_context: bool,
}

/// Resolves names and validates static semantics for a parsed source file.
#[must_use]
pub fn analyze(file: &str, ast: &Ast) -> Analysis {
    let mut analyzer = Analyzer::new(file, ast);
    analyzer.collect_declarations();
    analyzer.resolve_declarations();
    analyzer.check_bodies();
    analyzer
        .diagnostics
        .extend(analyze_ownership(file, ast).diagnostics);
    Analysis {
        symbols: analyzer.symbols,
        diagnostics: analyzer.diagnostics,
        expression_types: analyzer
            .expression_types
            .into_iter()
            .map(|(span, ty)| ExpressionType { span, ty })
            .collect(),
    }
}

fn analyze_with_imports(file: &str, ast: &Ast, imports: &[(&str, &Item)]) -> Analysis {
    let mut analyzer = Analyzer::new(file, ast);
    analyzer.collect_declarations();
    for (alias, item) in imports {
        analyzer.install_import(alias, item);
    }
    analyzer.resolve_declarations();
    analyzer.check_bodies();
    analyzer
        .diagnostics
        .extend(analyze_ownership(file, ast).diagnostics);
    Analysis {
        symbols: analyzer.symbols,
        diagnostics: analyzer.diagnostics,
        expression_types: analyzer
            .expression_types
            .into_iter()
            .map(|(span, ty)| ExpressionType { span, ty })
            .collect(),
    }
}

/// Parses, validates domain configuration, and runs semantic analysis.
#[must_use]
pub fn check_syl(file: impl Into<String>, source: &str) -> CheckResult {
    let file = file.into();
    let mut checked = syllog_parser::check_syl(file.clone(), source);
    if let Some(ast) = &checked.ast {
        checked.diagnostics.extend(analyze(&file, ast).diagnostics);
    }
    checked
}

impl<'a> Analyzer<'a> {
    fn new(file: &'a str, ast: &'a Ast) -> Self {
        let mut symbols = SymbolTable::default();
        for &name in primitive_names() {
            symbols.types.insert(
                name.to_owned(),
                TypeSymbol {
                    name: name.to_owned(),
                    kind: TypeSymbolKind::Primitive,
                    span: Span::default(),
                },
            );
        }
        for (name, arity) in [("Option", 1), ("Result", 2)] {
            symbols.types.insert(
                name.to_owned(),
                TypeSymbol {
                    name: name.to_owned(),
                    kind: TypeSymbolKind::Generic { arity },
                    span: Span::default(),
                },
            );
        }
        Self {
            file,
            ast,
            symbols,
            diagnostics: Vec::new(),
            expression_types: BTreeMap::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
            functions: HashMap::new(),
            agents: HashMap::new(),
            states: HashMap::new(),
            async_context: false,
        }
    }

    fn collect_declarations(&mut self) {
        for item in &self.ast.items {
            match item {
                Item::Struct(node) => {
                    self.insert_type(&node.name, TypeSymbolKind::Struct, node.span);
                }
                Item::Enum(node) => self.insert_type(&node.name, TypeSymbolKind::Enum, node.span),
                Item::State(node) => self.insert_type(&node.name, TypeSymbolKind::State, node.span),
                Item::Function(node) => {
                    self.insert_value(&node.name, ValueSymbolKind::Function, node.span);
                }
                Item::Agent(node) => {
                    self.insert_value(&node.name, ValueSymbolKind::Agent, node.span);
                }
                Item::Pipeline(node) => {
                    self.insert_value(&node.name, ValueSymbolKind::Pipeline, node.span);
                }
                Item::SafetyBound(node) => {
                    self.insert_value(&node.name, ValueSymbolKind::SafetyBound, node.span);
                }
            }
        }
    }

    fn install_import(&mut self, alias: &str, item: &Item) {
        match item {
            Item::Struct(node) => {
                self.insert_type(alias, TypeSymbolKind::Struct, node.span);
                let fields = node
                    .fields
                    .iter()
                    .map(|field| (field.name.clone(), self.resolve_type(&field.ty)))
                    .collect();
                self.structs
                    .insert(alias.to_owned(), StructDefinition { fields });
            }
            Item::Enum(node) => {
                self.insert_type(alias, TypeSymbolKind::Enum, node.span);
                let variants = node
                    .variants
                    .iter()
                    .map(|variant| {
                        (
                            variant.name.clone(),
                            variant
                                .fields
                                .iter()
                                .map(|field| self.resolve_type(field))
                                .collect(),
                        )
                    })
                    .collect();
                self.enums
                    .insert(alias.to_owned(), EnumDefinition { variants });
            }
            Item::Function(node) => {
                self.insert_value(alias, ValueSymbolKind::Function, node.span);
                let parameters = node
                    .parameters
                    .iter()
                    .map(|parameter| self.resolve_type(&parameter.ty))
                    .collect();
                let result = node
                    .return_type
                    .as_ref()
                    .map_or(ResolvedType::Unit, |ty| self.resolve_type(ty));
                self.functions
                    .insert(alias.to_owned(), FunctionSignature { parameters, result });
            }
            Item::State(node) => {
                self.insert_type(alias, TypeSymbolKind::State, node.span);
                let fields = node
                    .fields
                    .iter()
                    .map(|field| self.resolve_type(&field.ty))
                    .collect();
                self.states.insert(alias.to_owned(), fields);
            }
            Item::Agent(node) => {
                self.insert_value(alias, ValueSymbolKind::Agent, node.span);
                let input = property(&node.fields, "input")
                    .and_then(|field| field.ty.as_ref())
                    .map(|ty| self.resolve_type(ty));
                let output = property(&node.fields, "output")
                    .and_then(|field| field.ty.as_ref())
                    .map(|ty| self.resolve_type(ty));
                self.agents
                    .insert(alias.to_owned(), AgentContract { input, output });
            }
            Item::Pipeline(node) => {
                self.insert_value(alias, ValueSymbolKind::Pipeline, node.span);
                let parameters = node
                    .parameters
                    .iter()
                    .map(|parameter| self.resolve_type(&parameter.ty))
                    .collect();
                let result = node
                    .return_type
                    .as_ref()
                    .map_or(ResolvedType::Unit, |ty| self.resolve_type(ty));
                self.functions
                    .insert(alias.to_owned(), FunctionSignature { parameters, result });
            }
            Item::SafetyBound(node) => {
                self.insert_value(alias, ValueSymbolKind::SafetyBound, node.span);
            }
        }
    }

    fn insert_type(&mut self, name: &str, kind: TypeSymbolKind, span: Span) {
        if self.symbols.types.contains_key(name) {
            self.push_error("SYL2001", format!("duplicate type symbol '{name}'"), span);
            return;
        }
        self.symbols.types.insert(
            name.to_owned(),
            TypeSymbol {
                name: name.to_owned(),
                kind,
                span,
            },
        );
    }

    fn insert_value(&mut self, name: &str, kind: ValueSymbolKind, span: Span) {
        if self.symbols.values.contains_key(name) {
            self.push_error("SYL2001", format!("duplicate value symbol '{name}'"), span);
            return;
        }
        self.symbols.values.insert(
            name.to_owned(),
            ValueSymbol {
                name: name.to_owned(),
                kind,
                span,
            },
        );
    }

    fn resolve_declarations(&mut self) {
        for item in &self.ast.items {
            match item {
                Item::Struct(node) => self.resolve_struct(node),
                Item::Enum(node) => self.resolve_enum(node),
                Item::State(node) => self.resolve_state_signature(node),
                Item::Function(node) => self.resolve_function_signature(node),
                Item::Agent(node) => self.resolve_agent_contract(node),
                Item::Pipeline(node) => self.resolve_pipeline_signature(node),
                Item::SafetyBound(node) => {
                    for parameter in &node.parameters {
                        self.resolve_type(&parameter.ty);
                    }
                    for field in &node.fields {
                        if let Some(ty) = &field.ty {
                            self.resolve_type(ty);
                        }
                    }
                }
            }
        }
    }

    fn resolve_struct(&mut self, node: &StructNode) {
        if !matches!(
            self.symbols
                .types
                .get(&node.name)
                .map(|symbol| &symbol.kind),
            Some(TypeSymbolKind::Struct)
        ) {
            return;
        }
        let fields = node
            .fields
            .iter()
            .map(|field| (field.name.clone(), self.resolve_type(&field.ty)))
            .collect();
        self.structs
            .insert(node.name.clone(), StructDefinition { fields });
    }

    fn resolve_enum(&mut self, node: &EnumNode) {
        if !matches!(
            self.symbols
                .types
                .get(&node.name)
                .map(|symbol| &symbol.kind),
            Some(TypeSymbolKind::Enum)
        ) {
            return;
        }
        let variants = node
            .variants
            .iter()
            .map(|variant| {
                (
                    variant.name.clone(),
                    variant
                        .fields
                        .iter()
                        .map(|field| self.resolve_type(field))
                        .collect(),
                )
            })
            .collect();
        self.enums
            .insert(node.name.clone(), EnumDefinition { variants });
    }

    fn resolve_state_signature(&mut self, node: &StateNode) {
        let fields = node
            .fields
            .iter()
            .map(|field| self.resolve_type(&field.ty))
            .collect();
        self.states.entry(node.name.clone()).or_insert(fields);
    }

    fn resolve_function_signature(&mut self, node: &FunctionNode) {
        if !matches!(
            self.symbols
                .values
                .get(&node.name)
                .map(|symbol| &symbol.kind),
            Some(ValueSymbolKind::Function)
        ) || self.functions.contains_key(&node.name)
        {
            return;
        }
        let parameters = node
            .parameters
            .iter()
            .map(|parameter| self.resolve_type(&parameter.ty))
            .collect();
        let result = node
            .return_type
            .as_ref()
            .map_or(ResolvedType::Unit, |ty| self.resolve_type(ty));
        self.functions
            .insert(node.name.clone(), FunctionSignature { parameters, result });
    }

    fn resolve_agent_contract(&mut self, node: &AgentNode) {
        if self.agents.contains_key(&node.name) {
            return;
        }
        let input = property(node.fields.as_slice(), "input")
            .and_then(|field| field.ty.as_ref())
            .map(|ty| self.resolve_type(ty));
        let output = property(node.fields.as_slice(), "output")
            .and_then(|field| field.ty.as_ref())
            .map(|ty| self.resolve_type(ty));
        for field in &node.fields {
            if !matches!(field.name.as_str(), "input" | "output")
                && let Some(ty) = &field.ty
            {
                self.resolve_type(ty);
            }
        }
        self.agents
            .insert(node.name.clone(), AgentContract { input, output });
    }

    fn resolve_pipeline_signature(&mut self, node: &PipelineNode) {
        let parameters: Vec<_> = node
            .parameters
            .iter()
            .map(|parameter| self.resolve_type(&parameter.ty))
            .collect();
        let result = node
            .return_type
            .as_ref()
            .map_or(ResolvedType::Unit, |ty| self.resolve_type(ty));
        self.functions
            .entry(node.name.clone())
            .or_insert(FunctionSignature { parameters, result });
        for field in &node.fields {
            if let Some(ty) = &field.ty {
                self.resolve_type(ty);
            }
        }
    }

    fn resolve_type(&mut self, node: &TypeNode) -> ResolvedType {
        match &node.kind {
            TypeKind::Reference {
                lifetime,
                mutable,
                inner,
            } => ResolvedType::Reference {
                region: lifetime.clone(),
                mutable: *mutable,
                inner: Box::new(self.resolve_type(inner)),
            },
            TypeKind::Array(inner) => ResolvedType::Array(Box::new(self.resolve_type(inner))),
            TypeKind::Tuple(items) if items.is_empty() => ResolvedType::Unit,
            TypeKind::Tuple(items) => {
                ResolvedType::Tuple(items.iter().map(|item| self.resolve_type(item)).collect())
            }
            TypeKind::Path {
                segments,
                arguments,
            } => {
                let name = segments.join("::");
                if segments.len() != 1 {
                    self.push_error("SYL2002", format!("unknown type '{name}'"), node.span);
                    return ResolvedType::Error;
                }
                let name = &segments[0];
                match name.as_str() {
                    "Option" => {
                        if arguments.len() != 1 {
                            self.type_arity_error(name, 1, arguments.len(), node.span);
                            return ResolvedType::Error;
                        }
                        ResolvedType::Option(Box::new(self.resolve_type(&arguments[0])))
                    }
                    "Result" => {
                        if arguments.len() != 2 {
                            self.type_arity_error(name, 2, arguments.len(), node.span);
                            return ResolvedType::Error;
                        }
                        ResolvedType::Result(
                            Box::new(self.resolve_type(&arguments[0])),
                            Box::new(self.resolve_type(&arguments[1])),
                        )
                    }
                    "Unit" if arguments.is_empty() => ResolvedType::Unit,
                    _ => {
                        let Some(symbol) = self.symbols.types.get(name) else {
                            self.push_error("SYL2002", format!("unknown type '{name}'"), node.span);
                            return ResolvedType::Error;
                        };
                        if !arguments.is_empty() {
                            self.type_arity_error(name, 0, arguments.len(), node.span);
                            return ResolvedType::Error;
                        }
                        match symbol.kind {
                            TypeSymbolKind::Primitive => primitive_type(name)
                                .map_or(ResolvedType::Error, ResolvedType::Primitive),
                            TypeSymbolKind::Struct => ResolvedType::Struct(name.clone()),
                            TypeSymbolKind::Enum => ResolvedType::Enum(name.clone()),
                            TypeSymbolKind::State => ResolvedType::State(name.clone()),
                            TypeSymbolKind::Generic { .. } => ResolvedType::Error,
                        }
                    }
                }
            }
        }
    }

    fn type_arity_error(&mut self, name: &str, expected: usize, actual: usize, span: Span) {
        self.push_error(
            "SYL2004",
            format!("type '{name}' expects {expected} argument(s), found {actual}"),
            span,
        );
    }

    fn check_bodies(&mut self) {
        for item in &self.ast.items {
            match item {
                Item::Function(node) => self.check_function(node),
                Item::State(node) => self.check_state(node),
                Item::Pipeline(node) => self.check_pipeline(node),
                _ => {}
            }
        }
    }

    fn check_function(&mut self, node: &FunctionNode) {
        let Some(signature) = self.functions.get(&node.name).cloned() else {
            return;
        };
        let mut scope = HashMap::new();
        for (parameter, ty) in node.parameters.iter().zip(&signature.parameters) {
            scope.insert(parameter.name.clone(), ty.clone());
        }
        let previous = self.async_context;
        self.async_context = node.asynchronous;
        self.check_block(&node.body, &mut scope, &signature.result);
        self.async_context = previous;
    }

    fn check_state(&mut self, node: &StateNode) {
        let resolved = self.states.get(&node.name).cloned().unwrap_or_default();
        for (field, expected) in node.fields.iter().zip(resolved) {
            if let Some(initializer) = &field.initializer {
                let actual = self.infer_expr(initializer, &mut HashMap::new(), Some(&expected));
                self.require_compatible(&expected, &actual, initializer.span, "state initializer");
            }
        }
    }

    fn check_pipeline(&mut self, node: &PipelineNode) {
        let signature = self
            .functions
            .get(&node.name)
            .cloned()
            .unwrap_or(FunctionSignature {
                parameters: Vec::new(),
                result: ResolvedType::Unit,
            });
        if let Some(agent_name) = property(&node.fields, "agent").and_then(property_identifier)
            && let Some(contract) = self.agents.get(agent_name).cloned()
        {
            if let Some(agent_input) = contract.input {
                let pipeline_input = signature.parameters.first();
                if !pipeline_input.is_some_and(|input| compatible(&agent_input, input)) {
                    self.push_error(
                        "SYL2201",
                        format!(
                            "pipeline input is incompatible with agent '{agent_name}': expected {agent_input}, found {}",
                            pipeline_input.map_or_else(|| ResolvedType::Unit.to_string(), ToString::to_string)
                        ),
                        node.parameters.first().map_or(node.span, |parameter| parameter.span),
                    );
                }
            }
            if let Some(agent_output) = contract.output
                && !compatible(&agent_output, &signature.result)
            {
                self.push_error(
                    "SYL2201",
                    format!(
                        "pipeline output is incompatible with agent '{agent_name}': expected {agent_output}, found {}",
                        signature.result
                    ),
                    node.return_type.as_ref().map_or(node.span, |ty| ty.span),
                );
            }
        }

        let mut scope = HashMap::new();
        for (parameter, ty) in node.parameters.iter().zip(&signature.parameters) {
            scope.insert(parameter.name.clone(), ty.clone());
        }
        if let Some(result) = property(&node.fields, "result") {
            let declared = result
                .ty
                .as_ref()
                .map_or_else(|| signature.result.clone(), |ty| self.resolve_type(ty));
            self.require_compatible(
                &signature.result,
                &declared,
                result.span,
                "pipeline result annotation",
            );
            let actual = self.infer_expr(&result.value, &mut scope, Some(&declared));
            self.require_compatible(&declared, &actual, result.value.span, "pipeline result");
        }
    }

    fn check_block(
        &mut self,
        block: &Block,
        scope: &mut HashMap<String, ResolvedType>,
        expected_result: &ResolvedType,
    ) -> ResolvedType {
        let mut result = ResolvedType::Unit;
        let last_index = block.statements.len().saturating_sub(1);
        for (index, statement) in block.statements.iter().enumerate() {
            match &statement.kind {
                StatementKind::Let { name, ty, value } => {
                    let declared = ty.as_ref().map(|ty| self.resolve_type(ty));
                    let actual = self.infer_expr(value, scope, declared.as_ref());
                    let bound = declared.unwrap_or(actual);
                    scope.insert(name.clone(), bound);
                }
                StatementKind::Return(value) => {
                    let actual = value.as_ref().map_or(ResolvedType::Unit, |value| {
                        self.infer_expr(value, scope, Some(expected_result))
                    });
                    self.require_compatible(
                        expected_result,
                        &actual,
                        statement.span,
                        "return value",
                    );
                }
                StatementKind::Expression(expression) => {
                    let expected = (index == last_index).then_some(expected_result);
                    result = self.infer_expr(expression, scope, expected);
                    if index == last_index {
                        self.require_compatible(
                            expected_result,
                            &result,
                            expression.span,
                            "function result",
                        );
                    }
                }
            }
        }
        if block.statements.is_empty() {
            self.require_compatible(
                expected_result,
                &ResolvedType::Unit,
                block.span,
                "function result",
            );
        }
        result
    }

    fn infer_expr(
        &mut self,
        expression: &Expr,
        scope: &mut HashMap<String, ResolvedType>,
        expected: Option<&ResolvedType>,
    ) -> ResolvedType {
        let inferred = self.infer_expr_kind(expression, scope, expected);
        self.expression_types
            .insert(expression.span, inferred.clone());
        inferred
    }

    fn infer_expr_kind(
        &mut self,
        expression: &Expr,
        scope: &mut HashMap<String, ResolvedType>,
        expected: Option<&ResolvedType>,
    ) -> ResolvedType {
        match &expression.kind {
            ExprKind::Borrow { mutable, operand } => self.infer_borrow(*mutable, operand, scope),
            ExprKind::Await(operand) => {
                if !self.async_context {
                    self.push_error(
                        "SYL2501",
                        "'await' is only valid inside an async function",
                        expression.span,
                    );
                }
                self.infer_expr(operand, scope, expected)
            }
            ExprKind::Literal(literal) => {
                self.infer_literal(literal, expression.span, scope, expected)
            }
            ExprKind::Path(segments) => self.infer_path(segments, expression.span),
            ExprKind::Array(items) => {
                let expected_item = match expected {
                    Some(ResolvedType::Array(item)) => Some(item.as_ref()),
                    _ => None,
                };
                let mut item_type = expected_item.cloned().unwrap_or(ResolvedType::Unknown);
                for item in items {
                    let actual = self.infer_expr(item, scope, expected_item);
                    if item_type == ResolvedType::Unknown {
                        item_type = actual;
                    } else {
                        self.require_compatible(&item_type, &actual, item.span, "array element");
                    }
                }
                ResolvedType::Array(Box::new(item_type))
            }
            ExprKind::Call { callee, arguments } => {
                self.infer_call(callee, arguments, expression.span, scope, expected)
            }
            ExprKind::Field { base, name } => {
                let base_type = self.infer_expr(base, scope, None);
                if let ResolvedType::Struct(struct_name) = base_type {
                    if let Some(field) = self
                        .structs
                        .get(&struct_name)
                        .and_then(|definition| definition.fields.get(name))
                    {
                        return field.clone();
                    }
                    self.push_error(
                        "SYL2003",
                        format!("type '{struct_name}' has no field '{name}'"),
                        expression.span,
                    );
                }
                ResolvedType::Error
            }
            ExprKind::Unary { operator, operand } => {
                let operand_type = self.infer_expr(operand, scope, None);
                match operator {
                    syllog_parser::UnaryOperator::Not => {
                        self.require_compatible(
                            &bool_type(),
                            &operand_type,
                            operand.span,
                            "'!' operand",
                        );
                        bool_type()
                    }
                    syllog_parser::UnaryOperator::Negate => operand_type,
                }
            }
            ExprKind::Binary {
                operator,
                left,
                right,
            } => self.infer_binary(*operator, left, right, scope),
            ExprKind::Match { value, arms } => {
                let value_type = self.infer_expr(value, scope, None);
                self.check_match(expression.span, &value_type, arms, scope, expected)
            }
            ExprKind::Block(block) => {
                let mut nested = scope.clone();
                self.check_block(
                    block,
                    &mut nested,
                    expected.unwrap_or(&ResolvedType::Unknown),
                )
            }
        }
    }

    fn infer_borrow(
        &mut self,
        mutable: bool,
        operand: &Expr,
        scope: &mut HashMap<String, ResolvedType>,
    ) -> ResolvedType {
        if !matches!(
            operand.kind,
            ExprKind::Path(_) | ExprKind::Field { .. } | ExprKind::Literal(Literal::Identifier(_))
        ) {
            self.push_error(
                "SYL2601",
                "borrow operand must be a named place",
                operand.span,
            );
        }
        ResolvedType::Reference {
            region: None,
            mutable,
            inner: Box::new(self.infer_expr(operand, scope, None)),
        }
    }

    fn infer_literal(
        &mut self,
        literal: &Literal,
        span: Span,
        scope: &HashMap<String, ResolvedType>,
        expected: Option<&ResolvedType>,
    ) -> ResolvedType {
        match literal {
            Literal::String(_) => string_type(),
            Literal::Boolean(_) => bool_type(),
            Literal::Integer(_) => ResolvedType::IntegerLiteral,
            Literal::Float(_) => ResolvedType::FloatLiteral,
            Literal::Identifier(name) if name == "none" => {
                if let Some(ty @ (ResolvedType::Option(_) | ResolvedType::Error)) = expected {
                    ty.clone()
                } else {
                    self.push_error("SYL2003", "'none' requires an Option context", span);
                    ResolvedType::Error
                }
            }
            Literal::Identifier(name) => {
                if let Some(ty) = scope.get(name) {
                    return ty.clone();
                }
                if let Some(signature) = self.functions.get(name) {
                    return ResolvedType::Function(
                        signature.parameters.clone(),
                        Box::new(signature.result.clone()),
                    );
                }
                self.push_error("SYL2003", format!("unknown value '{name}'"), span);
                ResolvedType::Error
            }
        }
    }

    fn infer_path(&mut self, segments: &[String], span: Span) -> ResolvedType {
        if let Some((enum_name, variant_name)) = split_constructor_path(segments)
            && let Some(definition) = self.enums.get(enum_name)
            && let Some((_, fields)) = definition
                .variants
                .iter()
                .find(|(name, _)| name == variant_name)
        {
            if fields.is_empty() {
                return ResolvedType::Enum(enum_name.to_owned());
            }
            self.push_error(
                "SYL2101",
                format!(
                    "variant '{enum_name}::{variant_name}' requires {} value(s)",
                    fields.len()
                ),
                span,
            );
            return ResolvedType::Error;
        }
        self.push_error(
            "SYL2003",
            format!("unknown value path '{}'", segments.join("::")),
            span,
        );
        ResolvedType::Error
    }

    fn infer_call(
        &mut self,
        callee: &Expr,
        arguments: &[syllog_parser::CallArgument],
        span: Span,
        scope: &mut HashMap<String, ResolvedType>,
        expected: Option<&ResolvedType>,
    ) -> ResolvedType {
        if let ExprKind::Path(segments) = &callee.kind {
            return self.infer_constructor_call(segments, arguments, span, scope, expected);
        }
        let callee_type = self.infer_expr(callee, scope, None);
        let ResolvedType::Function(parameters, result) = callee_type else {
            if !matches!(callee_type, ResolvedType::Error) {
                self.push_error(
                    "SYL2101",
                    "called expression is not a function",
                    callee.span,
                );
            }
            for argument in arguments {
                self.infer_expr(&argument.value, scope, None);
            }
            return ResolvedType::Error;
        };
        if parameters.len() != arguments.len() {
            self.push_error(
                "SYL2101",
                format!(
                    "function expects {} argument(s), found {}",
                    parameters.len(),
                    arguments.len()
                ),
                span,
            );
        }
        for (argument, expected) in arguments.iter().zip(&parameters) {
            let actual = self.infer_expr(&argument.value, scope, Some(expected));
            self.require_compatible(expected, &actual, argument.span, "call argument");
        }
        *result
    }

    fn infer_constructor_call(
        &mut self,
        segments: &[String],
        arguments: &[syllog_parser::CallArgument],
        span: Span,
        scope: &mut HashMap<String, ResolvedType>,
        expected: Option<&ResolvedType>,
    ) -> ResolvedType {
        let Some((type_name, variant_name)) = split_constructor_path(segments) else {
            self.push_error("SYL2003", "unknown constructor", span);
            return ResolvedType::Error;
        };
        let (fields, result) = if type_name == "Option" {
            let expected_item = match expected {
                Some(ResolvedType::Option(item)) => (**item).clone(),
                _ => arguments.first().map_or(ResolvedType::Unknown, |argument| {
                    self.infer_expr(&argument.value, scope, None)
                }),
            };
            let fields = if variant_name == "some" {
                vec![expected_item.clone()]
            } else if variant_name == "none" {
                Vec::new()
            } else {
                self.unknown_variant(type_name, variant_name, span);
                return ResolvedType::Error;
            };
            (fields, ResolvedType::Option(Box::new(expected_item)))
        } else if type_name == "Result" {
            let (ok_type, error_type) = match expected {
                Some(ResolvedType::Result(ok, error)) => ((**ok).clone(), (**error).clone()),
                _ if variant_name == "ok" => (
                    arguments.first().map_or(ResolvedType::Unknown, |argument| {
                        self.infer_expr(&argument.value, scope, None)
                    }),
                    ResolvedType::Unknown,
                ),
                _ if variant_name == "err" => (
                    ResolvedType::Unknown,
                    arguments.first().map_or(ResolvedType::Unknown, |argument| {
                        self.infer_expr(&argument.value, scope, None)
                    }),
                ),
                _ => {
                    self.unknown_variant(type_name, variant_name, span);
                    return ResolvedType::Error;
                }
            };
            let fields = match variant_name {
                "ok" => vec![ok_type.clone()],
                "err" => vec![error_type.clone()],
                _ => {
                    self.unknown_variant(type_name, variant_name, span);
                    return ResolvedType::Error;
                }
            };
            (
                fields,
                ResolvedType::Result(Box::new(ok_type), Box::new(error_type)),
            )
        } else if let Some(definition) = self.enums.get(type_name) {
            let Some((_, fields)) = definition
                .variants
                .iter()
                .find(|(name, _)| name == variant_name)
                .cloned()
            else {
                self.unknown_variant(type_name, variant_name, span);
                return ResolvedType::Error;
            };
            (fields, ResolvedType::Enum(type_name.to_owned()))
        } else {
            self.push_error(
                "SYL2003",
                format!("unknown constructor '{type_name}::{variant_name}'"),
                span,
            );
            return ResolvedType::Error;
        };
        if fields.len() != arguments.len() {
            self.push_error(
                "SYL2101",
                format!(
                    "constructor expects {} value(s), found {}",
                    fields.len(),
                    arguments.len()
                ),
                span,
            );
        }
        for (argument, expected) in arguments.iter().zip(&fields) {
            let actual = self.infer_expr(&argument.value, scope, Some(expected));
            self.require_compatible(expected, &actual, argument.span, "constructor argument");
        }
        result
    }

    fn infer_binary(
        &mut self,
        operator: BinaryOperator,
        left: &Expr,
        right: &Expr,
        scope: &mut HashMap<String, ResolvedType>,
    ) -> ResolvedType {
        let left_type = self.infer_expr(left, scope, None);
        let right_type = self.infer_expr(right, scope, Some(&left_type));
        match operator {
            BinaryOperator::Equal
            | BinaryOperator::NotEqual
            | BinaryOperator::Less
            | BinaryOperator::LessEqual
            | BinaryOperator::Greater
            | BinaryOperator::GreaterEqual => {
                self.require_compatible(&left_type, &right_type, right.span, "comparison");
                bool_type()
            }
            BinaryOperator::And | BinaryOperator::Or => {
                self.require_compatible(&bool_type(), &left_type, left.span, "Boolean operator");
                self.require_compatible(&bool_type(), &right_type, right.span, "Boolean operator");
                bool_type()
            }
            _ => {
                self.require_compatible(&left_type, &right_type, right.span, "binary operator");
                left_type
            }
        }
    }

    fn check_match(
        &mut self,
        span: Span,
        value_type: &ResolvedType,
        arms: &[MatchArm],
        scope: &HashMap<String, ResolvedType>,
        expected: Option<&ResolvedType>,
    ) -> ResolvedType {
        let mut result = expected.cloned().unwrap_or(ResolvedType::Unknown);
        for arm in arms {
            let mut arm_scope = scope.clone();
            self.check_pattern(&arm.pattern, value_type, &mut arm_scope);
            if let Some(guard) = &arm.guard {
                let guard_type = self.infer_expr(guard, &mut arm_scope, Some(&bool_type()));
                self.require_compatible(&bool_type(), &guard_type, guard.span, "match guard");
            }
            let arm_type = self.infer_expr(&arm.body, &mut arm_scope, expected);
            if result == ResolvedType::Unknown {
                result = arm_type;
            } else {
                self.require_compatible(&result, &arm_type, arm.body.span, "match arm");
            }
        }
        self.check_exhaustive(span, value_type, arms);
        result
    }

    fn check_pattern(
        &mut self,
        pattern: &Pattern,
        expected: &ResolvedType,
        scope: &mut HashMap<String, ResolvedType>,
    ) {
        match &pattern.kind {
            PatternKind::Wildcard => {}
            PatternKind::Literal(literal) => {
                let actual = match literal {
                    Literal::Boolean(_) => bool_type(),
                    Literal::String(_) => string_type(),
                    Literal::Integer(_) => ResolvedType::IntegerLiteral,
                    Literal::Float(_) => ResolvedType::FloatLiteral,
                    Literal::Identifier(_) => ResolvedType::Unknown,
                };
                self.require_compatible(expected, &actual, pattern.span, "pattern");
            }
            PatternKind::Path(segments) if segments.len() == 1 => {
                scope.insert(segments[0].clone(), expected.clone());
            }
            PatternKind::Path(segments) => {
                self.check_variant_pattern(segments, &[], expected, scope, pattern.span);
            }
            PatternKind::Constructor { path, fields } => {
                self.check_variant_pattern(path, fields, expected, scope, pattern.span);
            }
        }
    }

    fn check_variant_pattern(
        &mut self,
        path: &[String],
        fields: &[Pattern],
        expected: &ResolvedType,
        scope: &mut HashMap<String, ResolvedType>,
        span: Span,
    ) {
        let Some((type_name, variant_name)) = split_constructor_path(path) else {
            self.push_error("SYL2003", "malformed variant pattern", span);
            return;
        };
        let payload = match expected {
            ResolvedType::Enum(enum_name) if enum_name == type_name => self
                .enums
                .get(enum_name)
                .and_then(|definition| {
                    definition
                        .variants
                        .iter()
                        .find(|(name, _)| name == variant_name)
                })
                .map(|(_, payload)| payload.clone()),
            ResolvedType::Option(inner) if type_name == "Option" => match variant_name {
                "some" => Some(vec![(**inner).clone()]),
                "none" => Some(Vec::new()),
                _ => None,
            },
            ResolvedType::Result(ok, error) if type_name == "Result" => match variant_name {
                "ok" => Some(vec![(**ok).clone()]),
                "err" => Some(vec![(**error).clone()]),
                _ => None,
            },
            ResolvedType::Error => return,
            _ => None,
        };
        let Some(payload) = payload else {
            self.unknown_variant(type_name, variant_name, span);
            return;
        };
        if payload.len() != fields.len() {
            self.push_error(
                "SYL2101",
                format!(
                    "pattern expects {} field(s), found {}",
                    payload.len(),
                    fields.len()
                ),
                span,
            );
        }
        for (field, field_type) in fields.iter().zip(&payload) {
            self.check_pattern(field, field_type, scope);
        }
    }

    fn check_exhaustive(&mut self, span: Span, value_type: &ResolvedType, arms: &[MatchArm]) {
        if arms.iter().any(|arm| {
            arm.guard.is_none()
                && match &arm.pattern.kind {
                    PatternKind::Wildcard => true,
                    PatternKind::Path(path) => path.len() == 1,
                    _ => false,
                }
        }) {
            return;
        }
        let required: Vec<String> = match value_type {
            ResolvedType::Enum(name) => self.enums.get(name).map_or_else(Vec::new, |definition| {
                definition
                    .variants
                    .iter()
                    .map(|(name, _)| name.clone())
                    .collect()
            }),
            ResolvedType::Option(_) => vec!["some".into(), "none".into()],
            ResolvedType::Result(_, _) => vec!["ok".into(), "err".into()],
            ResolvedType::Primitive(PrimitiveType::Bool) => vec!["true".into(), "false".into()],
            _ => return,
        };
        let covered: BTreeSet<String> = arms
            .iter()
            .filter(|arm| arm.guard.is_none())
            .filter_map(|arm| match &arm.pattern.kind {
                PatternKind::Constructor { path, .. } | PatternKind::Path(path) => {
                    path.last().cloned()
                }
                PatternKind::Literal(Literal::Boolean(value)) => Some(value.to_string()),
                _ => None,
            })
            .collect();
        let missing: Vec<_> = required
            .into_iter()
            .filter(|variant| !covered.contains(variant))
            .collect();
        if !missing.is_empty() {
            self.push_error(
                "SYL2301",
                format!("non-exhaustive match; missing {}", missing.join(", ")),
                span,
            );
        }
    }

    fn unknown_variant(&mut self, type_name: &str, variant_name: &str, span: Span) {
        self.push_error(
            "SYL2003",
            format!("type '{type_name}' has no variant '{variant_name}'"),
            span,
        );
    }

    fn require_compatible(
        &mut self,
        expected: &ResolvedType,
        actual: &ResolvedType,
        span: Span,
        context: &str,
    ) {
        if !compatible(expected, actual) {
            self.push_error(
                "SYL2101",
                format!("{context} type mismatch: expected {expected}, found {actual}"),
                span,
            );
        }
    }

    fn push_error(&mut self, code: &str, message: impl Into<String>, span: Span) {
        self.diagnostics.push(Diagnostic {
            code: code.to_owned(),
            severity: Severity::Error,
            message: message.into(),
            file: self.file.to_owned(),
            span,
        });
    }
}

fn property<'a>(fields: &'a [Property], name: &str) -> Option<&'a Property> {
    fields.iter().find(|field| field.name == name)
}

fn property_identifier(property: &Property) -> Option<&str> {
    match &property.value.kind {
        ExprKind::Literal(Literal::Identifier(name)) => Some(name),
        ExprKind::Path(path) => path.last().map(String::as_str),
        _ => None,
    }
}

fn split_constructor_path(segments: &[String]) -> Option<(&str, &str)> {
    (segments.len() == 2).then(|| (segments[0].as_str(), segments[1].as_str()))
}

fn compatible(expected: &ResolvedType, actual: &ResolvedType) -> bool {
    if matches!(expected, ResolvedType::Error | ResolvedType::Unknown)
        || matches!(actual, ResolvedType::Error | ResolvedType::Unknown)
    {
        return true;
    }
    match (expected, actual) {
        (
            ResolvedType::Primitive(PrimitiveType::Signed(_) | PrimitiveType::Unsigned(_)),
            ResolvedType::IntegerLiteral,
        )
        | (ResolvedType::Primitive(PrimitiveType::Float(_)), ResolvedType::FloatLiteral) => true,
        (ResolvedType::Array(left), ResolvedType::Array(right))
        | (ResolvedType::Option(left), ResolvedType::Option(right)) => compatible(left, right),
        (
            ResolvedType::Reference {
                mutable: left_mutable,
                inner: left,
                ..
            },
            ResolvedType::Reference {
                mutable: right_mutable,
                inner: right,
                ..
            },
        ) => left_mutable == right_mutable && compatible(left, right),
        (
            ResolvedType::Result(left_ok, left_error),
            ResolvedType::Result(right_ok, right_error),
        ) => compatible(left_ok, right_ok) && compatible(left_error, right_error),
        (ResolvedType::Tuple(left), ResolvedType::Tuple(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| compatible(left, right))
        }
        _ => expected == actual,
    }
}

fn primitive_names() -> &'static [&'static str] {
    &[
        "Bool", "String", "Str", "Char", "Bytes", "I8", "I16", "I32", "I64", "I128", "Isize", "U8",
        "U16", "U32", "U64", "U128", "Usize", "F16", "F32", "F64", "Duration", "Size", "AgentRef",
        "Provider", "Text", "Unit",
    ]
}

fn primitive_type(name: &str) -> Option<PrimitiveType> {
    Some(match name {
        "Bool" => PrimitiveType::Bool,
        "String" => PrimitiveType::String,
        "Str" => PrimitiveType::Str,
        "Char" => PrimitiveType::Char,
        "Bytes" => PrimitiveType::Bytes,
        "I8" | "I16" | "I32" | "I64" | "I128" | "Isize" => PrimitiveType::Signed(name.to_owned()),
        "U8" | "U16" | "U32" | "U64" | "U128" | "Usize" => PrimitiveType::Unsigned(name.to_owned()),
        "F16" | "F32" | "F64" => PrimitiveType::Float(name.to_owned()),
        "Duration" => PrimitiveType::Duration,
        "Size" => PrimitiveType::Size,
        "AgentRef" => PrimitiveType::AgentRef,
        "Provider" => PrimitiveType::Provider,
        "Text" => PrimitiveType::Text,
        _ => return None,
    })
}

fn bool_type() -> ResolvedType {
    ResolvedType::Primitive(PrimitiveType::Bool)
}

fn string_type() -> ResolvedType {
    ResolvedType::Primitive(PrimitiveType::String)
}
