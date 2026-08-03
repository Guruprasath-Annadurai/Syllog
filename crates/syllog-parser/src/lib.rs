//! Pest-based parser and span-aware typed syntax tree for Syllog source files.

mod ast;
mod diagnostic;
mod validate;

use anyhow::{Context, anyhow, bail};
use pest::Parser as _;
use pest::iterators::Pair;
use pest::pratt_parser::{Assoc, Op, PrattParser};
use std::collections::VecDeque;

pub use ast::*;
pub use diagnostic::{CheckResult, Diagnostic, Severity};

#[allow(missing_docs)]
mod generated {
    use pest_derive::Parser;

    #[derive(Parser)]
    #[grammar = "grammar.pest"]
    pub(super) struct SylParser;
}

use generated::{Rule, SylParser};

/// Parses a Syllog source file into its typed, source-positioned syntax tree.
///
/// # Errors
///
/// Returns a source-positioned Pest diagnostic for invalid syntax, or a
/// conversion error if a literal cannot be represented by the AST.
pub fn parse_syl(input: &str) -> anyhow::Result<Ast> {
    let mut parsed = SylParser::parse(Rule::program, input).map_err(|error| {
        anyhow!(
            "invalid Syllog source; expected struct, enum, fn, state, agent, pipeline, or safety_bound: {error}"
        )
    })?;
    let program = parsed
        .next()
        .context("parser did not produce a program node")?;
    lower_program(program)
}

fn lower_program(program: Pair<'_, Rule>) -> anyhow::Result<Ast> {
    let span = pair_span(&program);
    let items = program
        .into_inner()
        .filter(|pair| pair.as_rule() != Rule::EOI)
        .map(parse_item)
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(Ast { items, span })
}

/// Parses and validates one named Syllog source file.
#[must_use]
pub fn check_syl(file: impl Into<String>, input: &str) -> CheckResult {
    use pest::error::{InputLocation, LineColLocation};

    let file = file.into();
    let mut parsed = match SylParser::parse(Rule::program, input) {
        Ok(parsed) => parsed,
        Err(parse_error) => {
            let (start, end) = match parse_error.location {
                InputLocation::Pos(position) => (position, position),
                InputLocation::Span((start, end)) => (start, end),
            };
            let (line, column, end_line, end_column) = match parse_error.line_col {
                LineColLocation::Pos((line, column)) => (line, column, line, column),
                LineColLocation::Span((line, column), (end_line, end_column)) => {
                    (line, column, end_line, end_column)
                }
            };
            return CheckResult {
                ast: None,
                diagnostics: vec![Diagnostic {
                    code: "SYL0001".to_owned(),
                    severity: Severity::Error,
                    message: parse_error.variant.message().into_owned(),
                    file,
                    span: Span {
                        start,
                        end,
                        line,
                        column,
                        end_line,
                        end_column,
                    },
                }],
            };
        }
    };

    let lowered = parsed
        .next()
        .context("parser did not produce a program node")
        .and_then(lower_program);
    match lowered {
        Ok(ast) => CheckResult {
            diagnostics: validate::validate(&file, &ast),
            ast: Some(ast),
        },
        Err(error) => CheckResult {
            ast: None,
            diagnostics: vec![Diagnostic {
                code: "SYL0002".to_owned(),
                severity: Severity::Error,
                message: format!("internal AST lowering failure: {error:#}"),
                file,
                span: Span::default(),
            }],
        },
    }
}

fn parse_item(pair: Pair<'_, Rule>) -> anyhow::Result<Item> {
    match pair.as_rule() {
        Rule::struct_decl => parse_struct(pair).map(Item::Struct),
        Rule::enum_decl => parse_enum(pair).map(Item::Enum),
        Rule::function_decl => parse_function(pair).map(Item::Function),
        Rule::state_decl => parse_state(pair).map(Item::State),
        Rule::agent_decl => parse_agent(pair).map(Item::Agent),
        Rule::pipeline_decl => parse_pipeline(pair).map(Item::Pipeline),
        Rule::safety_bound_decl => parse_safety_bound(pair).map(Item::SafetyBound),
        unexpected => bail!("unexpected item rule: {unexpected:?}"),
    }
}

fn parse_struct(pair: Pair<'_, Rule>) -> anyhow::Result<StructNode> {
    let span = pair_span(&pair);
    let mut parts = meaningful_inner(pair);
    let public = take_marker(&mut parts, Rule::visibility);
    let name = take_identifier(&mut parts, "struct name")?;
    let fields = parts
        .into_iter()
        .map(parse_struct_field)
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(StructNode {
        public,
        name,
        fields,
        span,
    })
}

fn parse_struct_field(pair: Pair<'_, Rule>) -> anyhow::Result<StructField> {
    let span = pair_span(&pair);
    let mut parts = meaningful_inner(pair);
    let public = take_marker(&mut parts, Rule::visibility);
    let name = take_identifier(&mut parts, "struct field")?;
    let ty = parse_type(pop_front(&mut parts, "struct field type")?)?;
    Ok(StructField {
        public,
        name,
        ty,
        span,
    })
}

fn parse_enum(pair: Pair<'_, Rule>) -> anyhow::Result<EnumNode> {
    let span = pair_span(&pair);
    let mut parts = meaningful_inner(pair);
    let public = take_marker(&mut parts, Rule::visibility);
    let name = take_identifier(&mut parts, "enum name")?;
    let variants = parts
        .into_iter()
        .map(parse_enum_variant)
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(EnumNode {
        public,
        name,
        variants,
        span,
    })
}

fn parse_enum_variant(pair: Pair<'_, Rule>) -> anyhow::Result<EnumVariant> {
    let span = pair_span(&pair);
    let mut parts = meaningful_inner(pair);
    let name = take_identifier(&mut parts, "enum variant")?;
    let fields = parts
        .into_iter()
        .map(parse_type)
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(EnumVariant { name, fields, span })
}

fn parse_function(pair: Pair<'_, Rule>) -> anyhow::Result<FunctionNode> {
    let span = pair_span(&pair);
    let mut parts = meaningful_inner(pair);
    let mut attributes = Vec::new();
    while matches!(parts.front().map(Pair::as_rule), Some(Rule::attribute)) {
        let attribute = parts.pop_front().expect("front checked");
        let attribute_span = pair_span(&attribute);
        let mut attribute_parts = meaningful_inner(attribute);
        attributes.push(AttributeNode {
            name: take_identifier(&mut attribute_parts, "attribute name")?,
            span: attribute_span,
        });
    }
    let public = take_marker(&mut parts, Rule::visibility);
    let asynchronous = take_marker(&mut parts, Rule::async_marker);
    let name = take_identifier(&mut parts, "function name")?;
    let parameters = parse_parameters(pop_front(&mut parts, "function parameters")?)?;
    let return_type = if matches!(parts.front().map(Pair::as_rule), Some(Rule::return_type)) {
        Some(parse_return_type(
            parts.pop_front().expect("front checked"),
        )?)
    } else {
        None
    };
    let body = parse_block(pop_front(&mut parts, "function body")?)?;
    Ok(FunctionNode {
        attributes,
        public,
        asynchronous,
        name,
        parameters,
        return_type,
        body,
        span,
    })
}

fn parse_state(pair: Pair<'_, Rule>) -> anyhow::Result<StateNode> {
    let span = pair_span(&pair);
    let mut parts = meaningful_inner(pair);
    let public = take_marker(&mut parts, Rule::visibility);
    let name = take_identifier(&mut parts, "state name")?;
    let fields = parts
        .into_iter()
        .map(parse_state_field)
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(StateNode {
        public,
        name,
        fields,
        span,
    })
}

fn parse_state_field(pair: Pair<'_, Rule>) -> anyhow::Result<StateField> {
    let span = pair_span(&pair);
    let mut parts = meaningful_inner(pair);
    let name = take_identifier(&mut parts, "state field")?;
    let ty = parse_type(pop_front(&mut parts, "state field type")?)?;
    let initializer = parts.pop_front().map(parse_expression).transpose()?;
    Ok(StateField {
        name,
        ty,
        initializer,
        span,
    })
}

fn parse_agent(pair: Pair<'_, Rule>) -> anyhow::Result<AgentNode> {
    let span = pair_span(&pair);
    let mut parts = meaningful_inner(pair);
    let public = take_marker(&mut parts, Rule::visibility);
    let name = take_identifier(&mut parts, "agent name")?;
    let fields = parse_property_block(pop_front(&mut parts, "agent body")?)?;
    Ok(AgentNode {
        public,
        name,
        fields,
        span,
    })
}

fn parse_pipeline(pair: Pair<'_, Rule>) -> anyhow::Result<PipelineNode> {
    let span = pair_span(&pair);
    let mut parts = meaningful_inner(pair);
    let public = take_marker(&mut parts, Rule::visibility);
    let name = take_identifier(&mut parts, "pipeline name")?;
    let parameters = if matches!(parts.front().map(Pair::as_rule), Some(Rule::parameters)) {
        parse_parameters(parts.pop_front().expect("front checked"))?
    } else {
        Vec::new()
    };
    let return_type = if matches!(parts.front().map(Pair::as_rule), Some(Rule::return_type)) {
        Some(parse_return_type(
            parts.pop_front().expect("front checked"),
        )?)
    } else {
        None
    };
    let fields = parse_property_block(pop_front(&mut parts, "pipeline body")?)?;
    Ok(PipelineNode {
        public,
        name,
        parameters,
        return_type,
        fields,
        span,
    })
}

fn parse_safety_bound(pair: Pair<'_, Rule>) -> anyhow::Result<SafetyBoundNode> {
    let span = pair_span(&pair);
    let mut parts = meaningful_inner(pair);
    let public = take_marker(&mut parts, Rule::visibility);
    let name = take_identifier(&mut parts, "safety_bound name")?;
    let parameters = if matches!(parts.front().map(Pair::as_rule), Some(Rule::parameters)) {
        parse_parameters(parts.pop_front().expect("front checked"))?
    } else {
        Vec::new()
    };
    let fields = parse_property_block(pop_front(&mut parts, "safety_bound body")?)?;
    Ok(SafetyBoundNode {
        public,
        name,
        parameters,
        fields,
        span,
    })
}

fn parse_parameters(pair: Pair<'_, Rule>) -> anyhow::Result<Vec<Parameter>> {
    pair.into_inner().map(parse_parameter).collect()
}

fn parse_parameter(pair: Pair<'_, Rule>) -> anyhow::Result<Parameter> {
    let span = pair_span(&pair);
    let mut parts = meaningful_inner(pair);
    let name = take_identifier(&mut parts, "parameter")?;
    let ty = parse_type(pop_front(&mut parts, "parameter type")?)?;
    Ok(Parameter { name, ty, span })
}

fn parse_return_type(pair: Pair<'_, Rule>) -> anyhow::Result<TypeNode> {
    parse_type(pair.into_inner().next().context("return type is missing")?)
}

fn parse_property_block(pair: Pair<'_, Rule>) -> anyhow::Result<Vec<Property>> {
    pair.into_inner().map(parse_property).collect()
}

fn parse_property(pair: Pair<'_, Rule>) -> anyhow::Result<Property> {
    let span = pair_span(&pair);
    let mut parts = meaningful_inner(pair);
    let name = take_identifier(&mut parts, "property name")?;
    let value_or_typed = pop_front(&mut parts, "property value")?;
    let (ty, value) = if value_or_typed.as_rule() == Rule::typed_property {
        let mut typed = value_or_typed.into_inner();
        let ty = parse_type(typed.next().context("typed property type is missing")?)?;
        let value = parse_expression(typed.next().context("typed property value is missing")?)?;
        (Some(ty), value)
    } else {
        (None, parse_expression(value_or_typed)?)
    };
    Ok(Property {
        name,
        ty,
        value,
        span,
    })
}

fn parse_type(pair: Pair<'_, Rule>) -> anyhow::Result<TypeNode> {
    let pair = if pair.as_rule() == Rule::type_expr {
        pair.into_inner()
            .next()
            .context("type expression is empty")?
    } else {
        pair
    };
    let span = pair_span(&pair);
    let kind = match pair.as_rule() {
        Rule::array_type => {
            let inner = pair.into_inner().next().context("array type is empty")?;
            TypeKind::Array(Box::new(parse_type(inner)?))
        }
        Rule::tuple_type => TypeKind::Tuple(
            pair.into_inner()
                .map(parse_type)
                .collect::<anyhow::Result<Vec<_>>>()?,
        ),
        Rule::path_type => {
            let mut parts = pair.into_inner();
            let segments = parse_path(parts.next().context("type path is empty")?)?;
            let arguments = parts
                .next()
                .map(|generics| {
                    generics
                        .into_inner()
                        .map(parse_type)
                        .collect::<anyhow::Result<Vec<_>>>()
                })
                .transpose()?
                .unwrap_or_default();
            TypeKind::Path {
                segments,
                arguments,
            }
        }
        unexpected => bail!("unexpected type rule: {unexpected:?}"),
    };
    Ok(TypeNode { kind, span })
}

fn parse_block(pair: Pair<'_, Rule>) -> anyhow::Result<Block> {
    let span = pair_span(&pair);
    let statements = pair
        .into_inner()
        .map(parse_statement)
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(Block { statements, span })
}

fn parse_statement(pair: Pair<'_, Rule>) -> anyhow::Result<Statement> {
    let span = pair_span(&pair);
    let kind = match pair.as_rule() {
        Rule::let_statement => {
            let mut parts = meaningful_inner(pair);
            let name = take_identifier(&mut parts, "let binding")?;
            let ty = if matches!(parts.front().map(Pair::as_rule), Some(Rule::type_expr)) {
                Some(parse_type(parts.pop_front().expect("front checked"))?)
            } else {
                None
            };
            let value = parse_expression(pop_front(&mut parts, "let initializer")?)?;
            StatementKind::Let { name, ty, value }
        }
        Rule::return_statement => StatementKind::Return(
            meaningful_inner(pair)
                .into_iter()
                .next()
                .map(parse_expression)
                .transpose()?,
        ),
        Rule::expression_statement => StatementKind::Expression(parse_expression(
            pair.into_inner()
                .next()
                .context("expression statement is empty")?,
        )?),
        unexpected => bail!("unexpected statement rule: {unexpected:?}"),
    };
    Ok(Statement { kind, span })
}

fn parse_expression(pair: Pair<'_, Rule>) -> anyhow::Result<Expr> {
    if pair.as_rule() != Rule::expression {
        bail!("expected expression, found {:?}", pair.as_rule());
    }

    PrattParser::new()
        .op(Op::infix(Rule::op_or, Assoc::Left))
        .op(Op::infix(Rule::op_and, Assoc::Left))
        .op(Op::infix(Rule::op_equal, Assoc::Left) | Op::infix(Rule::op_not_equal, Assoc::Left))
        .op(Op::infix(Rule::op_less, Assoc::Left)
            | Op::infix(Rule::op_less_equal, Assoc::Left)
            | Op::infix(Rule::op_greater, Assoc::Left)
            | Op::infix(Rule::op_greater_equal, Assoc::Left))
        .op(Op::infix(Rule::op_add, Assoc::Left) | Op::infix(Rule::op_subtract, Assoc::Left))
        .op(Op::infix(Rule::op_multiply, Assoc::Left)
            | Op::infix(Rule::op_divide, Assoc::Left)
            | Op::infix(Rule::op_remainder, Assoc::Left))
        .op(Op::prefix(Rule::prefix_operator))
        .map_primary(parse_postfix)
        .map_prefix(|operator, operand| {
            let operand = operand?;
            let span = span_join(pair_span(&operator), operand.span);
            let operator = match operator
                .into_inner()
                .next()
                .context("prefix operator is empty")?
                .as_rule()
            {
                Rule::op_not => UnaryOperator::Not,
                Rule::op_negate => UnaryOperator::Negate,
                unexpected => bail!("unexpected prefix operator: {unexpected:?}"),
            };
            Ok(Expr {
                kind: ExprKind::Unary {
                    operator,
                    operand: Box::new(operand),
                },
                span,
            })
        })
        .map_infix(|left, operator, right| {
            let left = left?;
            let right = right?;
            let span = span_join(left.span, right.span);
            let operator = binary_operator(operator.as_rule())?;
            Ok(Expr {
                kind: ExprKind::Binary {
                    operator,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                span,
            })
        })
        .parse(pair.into_inner())
}

fn parse_postfix(pair: Pair<'_, Rule>) -> anyhow::Result<Expr> {
    let mut parts = meaningful_inner(pair).into_iter();
    let primary = parts.next().context("postfix expression is empty")?;
    let mut expression = parse_primary(primary)?;
    for suffix in parts {
        let span = span_join(expression.span, pair_span(&suffix));
        expression = match suffix.as_rule() {
            Rule::call_suffix => Expr {
                kind: ExprKind::Call {
                    callee: Box::new(expression),
                    arguments: suffix
                        .into_inner()
                        .map(parse_call_argument)
                        .collect::<anyhow::Result<Vec<_>>>()?,
                },
                span,
            },
            Rule::field_suffix => Expr {
                kind: ExprKind::Field {
                    base: Box::new(expression),
                    name: suffix
                        .into_inner()
                        .next()
                        .context("field suffix has no name")?
                        .as_str()
                        .to_owned(),
                },
                span,
            },
            unexpected => bail!("unexpected postfix suffix: {unexpected:?}"),
        };
    }
    Ok(expression)
}

fn parse_primary(pair: Pair<'_, Rule>) -> anyhow::Result<Expr> {
    let span = pair_span(&pair);
    let kind = match pair.as_rule() {
        Rule::string | Rule::boolean | Rule::float | Rule::integer => {
            ExprKind::Literal(parse_literal(&pair)?)
        }
        Rule::path_expression => {
            let segments = parse_path(
                pair.into_inner()
                    .next()
                    .context("path expression is empty")?,
            )?;
            if segments.len() == 1 {
                ExprKind::Literal(Literal::Identifier(
                    segments.into_iter().next().expect("length checked"),
                ))
            } else {
                ExprKind::Path(segments)
            }
        }
        Rule::array_expression => ExprKind::Array(
            pair.into_inner()
                .map(parse_expression)
                .collect::<anyhow::Result<Vec<_>>>()?,
        ),
        Rule::parenthesized => {
            return parse_expression(
                pair.into_inner()
                    .next()
                    .context("parenthesized expression is empty")?,
            );
        }
        Rule::match_expression => return parse_match(pair),
        Rule::block => ExprKind::Block(parse_block(pair)?),
        unexpected => bail!("unexpected primary rule: {unexpected:?}"),
    };
    Ok(Expr { kind, span })
}

fn parse_call_argument(pair: Pair<'_, Rule>) -> anyhow::Result<CallArgument> {
    let span = pair_span(&pair);
    let inner = pair.into_inner().next().context("call argument is empty")?;
    let (name, value) = if inner.as_rule() == Rule::named_argument {
        let mut parts = inner.into_inner();
        let name = parts
            .next()
            .context("named argument has no name")?
            .as_str()
            .to_owned();
        let value = parse_expression(parts.next().context("named argument has no value")?)?;
        (Some(name), value)
    } else {
        (None, parse_expression(inner)?)
    };
    Ok(CallArgument { name, value, span })
}

fn parse_match(pair: Pair<'_, Rule>) -> anyhow::Result<Expr> {
    let span = pair_span(&pair);
    let mut parts = meaningful_inner(pair).into_iter();
    let value = parse_expression(parts.next().context("match value is missing")?)?;
    let arms = parts.map(parse_match_arm).collect::<anyhow::Result<_>>()?;
    Ok(Expr {
        kind: ExprKind::Match {
            value: Box::new(value),
            arms,
        },
        span,
    })
}

fn parse_match_arm(pair: Pair<'_, Rule>) -> anyhow::Result<MatchArm> {
    let span = pair_span(&pair);
    let mut parts = meaningful_inner(pair);
    let pattern = parse_pattern(pop_front(&mut parts, "match pattern")?)?;
    let guard = if matches!(parts.front().map(Pair::as_rule), Some(Rule::match_guard)) {
        let guard = parts.pop_front().expect("front checked");
        Some(parse_expression(
            meaningful_inner(guard)
                .into_iter()
                .next()
                .context("match guard is empty")?,
        )?)
    } else {
        None
    };
    let body = parse_expression(pop_front(&mut parts, "match arm body")?)?;
    Ok(MatchArm {
        pattern,
        guard,
        body,
        span,
    })
}

fn parse_pattern(pair: Pair<'_, Rule>) -> anyhow::Result<Pattern> {
    let span = pair_span(&pair);
    let inner = pair.into_inner().next().context("pattern is empty")?;
    let kind = match inner.as_rule() {
        Rule::wildcard_pattern => PatternKind::Wildcard,
        Rule::path_pattern => PatternKind::Path(parse_path(
            inner.into_inner().next().context("path pattern is empty")?,
        )?),
        Rule::constructor_pattern => {
            let mut parts = inner.into_inner();
            let path = parse_path(parts.next().context("constructor path is empty")?)?;
            let fields = parts.map(parse_pattern).collect::<anyhow::Result<_>>()?;
            PatternKind::Constructor { path, fields }
        }
        Rule::literal_pattern => {
            let literal = inner
                .into_inner()
                .next()
                .context("literal pattern is empty")?;
            PatternKind::Literal(parse_literal(&literal)?)
        }
        unexpected => bail!("unexpected pattern rule: {unexpected:?}"),
    };
    Ok(Pattern { kind, span })
}

fn parse_literal(pair: &Pair<'_, Rule>) -> anyhow::Result<Literal> {
    match pair.as_rule() {
        Rule::string => serde_json::from_str(pair.as_str())
            .map(Literal::String)
            .context("invalid string literal"),
        Rule::integer => pair
            .as_str()
            .parse()
            .map(Literal::Integer)
            .context("integer literal is outside the signed 64-bit range"),
        Rule::float => Ok(Literal::Float(pair.as_str().to_owned())),
        Rule::boolean => Ok(Literal::Boolean(pair.as_str() == "true")),
        unexpected => bail!("unexpected literal rule: {unexpected:?}"),
    }
}

fn parse_path(pair: Pair<'_, Rule>) -> anyhow::Result<Vec<String>> {
    if pair.as_rule() != Rule::path {
        bail!("expected path, found {:?}", pair.as_rule());
    }
    Ok(pair
        .into_inner()
        .map(|segment| segment.as_str().to_owned())
        .collect())
}

fn binary_operator(rule: Rule) -> anyhow::Result<BinaryOperator> {
    Ok(match rule {
        Rule::op_add => BinaryOperator::Add,
        Rule::op_subtract => BinaryOperator::Subtract,
        Rule::op_multiply => BinaryOperator::Multiply,
        Rule::op_divide => BinaryOperator::Divide,
        Rule::op_remainder => BinaryOperator::Remainder,
        Rule::op_equal => BinaryOperator::Equal,
        Rule::op_not_equal => BinaryOperator::NotEqual,
        Rule::op_less => BinaryOperator::Less,
        Rule::op_less_equal => BinaryOperator::LessEqual,
        Rule::op_greater => BinaryOperator::Greater,
        Rule::op_greater_equal => BinaryOperator::GreaterEqual,
        Rule::op_and => BinaryOperator::And,
        Rule::op_or => BinaryOperator::Or,
        unexpected => bail!("unexpected binary operator: {unexpected:?}"),
    })
}

fn take_marker(parts: &mut VecDeque<Pair<'_, Rule>>, rule: Rule) -> bool {
    if matches!(parts.front().map(Pair::as_rule), Some(found) if found == rule) {
        parts.pop_front();
        true
    } else {
        false
    }
}

fn meaningful_inner(pair: Pair<'_, Rule>) -> VecDeque<Pair<'_, Rule>> {
    pair.into_inner()
        .filter(|inner| !is_keyword(inner.as_rule()))
        .collect()
}

fn is_keyword(rule: Rule) -> bool {
    matches!(
        rule,
        Rule::kw_struct
            | Rule::kw_enum
            | Rule::kw_fn
            | Rule::kw_state
            | Rule::kw_agent
            | Rule::kw_pipeline
            | Rule::kw_safety_bound
            | Rule::kw_let
            | Rule::kw_return
            | Rule::kw_match
            | Rule::kw_if
    )
}

fn take_identifier(parts: &mut VecDeque<Pair<'_, Rule>>, context: &str) -> anyhow::Result<String> {
    let pair = pop_front(parts, context)?;
    if pair.as_rule() != Rule::identifier {
        bail!("{context}: expected identifier, found {:?}", pair.as_rule());
    }
    Ok(pair.as_str().to_owned())
}

fn pop_front<'i>(
    parts: &mut VecDeque<Pair<'i, Rule>>,
    context: &str,
) -> anyhow::Result<Pair<'i, Rule>> {
    parts
        .pop_front()
        .with_context(|| format!("{context} is missing"))
}

fn pair_span(pair: &Pair<'_, Rule>) -> Span {
    let pest_span = pair.as_span();
    let (line, column) = pest_span.start_pos().line_col();
    let (end_line, end_column) = pest_span.end_pos().line_col();
    Span {
        start: pest_span.start(),
        end: pest_span.end(),
        line,
        column,
        end_line,
        end_column,
    }
}

fn span_join(left: Span, right: Span) -> Span {
    Span {
        start: left.start,
        end: right.end,
        line: left.line,
        column: left.column,
        end_line: right.end_line,
        end_column: right.end_column,
    }
}
