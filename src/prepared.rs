//! Prepared queries: parsed and lowered to decypher's HIR once, then executed
//! any number of times.

use std::ops::Range;

use decypher::ast::clause::Return;
use decypher::ast::expr::Expression;
use decypher::ast::query::{QueryBody, SinglePartBody, SingleQuery, SingleQueryKind};
use decypher::ast::{RegularQuery, ToCypher};
use decypher::hir::arena::{Id, Interner};
use decypher::hir::ops::Operation;
use decypher::hir::{BindingId, HirQuery, LowerConfig};

use crate::error::CypherError;
use crate::exec::functions::Function;

/// A Cypher query, parsed and analyzed once.
///
/// ```
/// use petgraph::Graph;
/// use petgraph_decypher::{EdgeData, NodeData, Parameters, Query, Value};
///
/// let mut graph = Graph::<NodeData, EdgeData>::new();
/// let create = Query::new("CREATE (:Person {name: $name})")?;
/// for name in ["Alice", "Bob"] {
///     let params = Parameters::from([("name".to_owned(), Value::from(name))]);
///     create.execute_mut(&mut graph, &params)?;
/// }
///
/// let names = Query::new("MATCH (p:Person) RETURN p.name AS name ORDER BY name")?
///     .execute(&graph, &Parameters::new())?;
/// assert_eq!(names.column("name").collect::<Vec<_>>(), [&Value::from("Alice"), &Value::from("Bob")]);
/// # Ok::<(), petgraph_decypher::CypherError>(())
/// ```
#[derive(Debug, Clone)]
pub struct Query {
    pub(crate) hir: HirQuery,
    pub(crate) labels: Vec<String>,
    pub(crate) rel_types: Vec<String>,
    pub(crate) keys: Vec<String>,
    pub(crate) parameters: Vec<String>,
    pub(crate) functions: Vec<Function>,
    pub(crate) statements: Vec<Statement>,
    /// Per expression: a comparison continuing the chain of the comparison
    /// on its left (`a < b < c`), see [`chained_comparisons`].
    pub(crate) chained: Vec<bool>,
    read_only: bool,
}

/// One `;`-separated statement: one branch, or several joined by `UNION`.
#[derive(Debug, Clone)]
pub(crate) struct Statement {
    pub branches: Vec<Branch>,
    /// `UNION` (deduplicating) rather than `UNION ALL`.
    pub distinct: bool,
}

/// A single query: a run of [`HirQuery::parts`] and the columns it returns.
#[derive(Debug, Clone)]
pub(crate) struct Branch {
    pub parts: Range<usize>,
    pub columns: Vec<String>,
    /// The binding holding each column.
    pub outputs: Vec<BindingId>,
    /// Whether the branch ends with `RETURN`; otherwise it yields no rows.
    pub returns: bool,
}

impl Query {
    /// Parses and analyzes `text`.
    pub fn new(text: &str) -> Result<Self, CypherError> {
        let ast = decypher::parse(text).map_err(|e| CypherError::ParseError(e.to_string()))?;
        let cst = decypher::parse_cst(text).tree;

        let mut catalog = decypher::hir::Catalog::default();
        catalog.set_strict(true);
        let config = LowerConfig { catalog };
        let mut hir = decypher::hir::lower::lower(&ast, &config)
            .map_err(|d| CypherError::InvalidQuery(d.to_string()))?;
        fix_pattern_offsets(&ast, &mut hir)?;

        let functions = names(&hir.arenas.functions)
            .iter()
            .map(|name| Function::resolve(name))
            .collect::<Result<Vec<_>, _>>()?;

        let mut statements = Vec::new();
        let mut next_part = 0;
        for body in &ast.statements {
            let singles: Vec<(&SingleQuery, bool)> = match body {
                QueryBody::SingleQuery(sq) => vec![(sq, true)],
                QueryBody::Regular(RegularQuery {
                    single_query,
                    unions,
                }) => std::iter::once((single_query, true))
                    .chain(unions.iter().map(|u| (&u.single_query, u.all)))
                    .collect(),
                QueryBody::Standalone(_) => {
                    return Err(CypherError::Unsupported("procedure calls".into()));
                }
                _ => return Err(CypherError::Unsupported("schema and admin commands".into())),
            };
            let distinct = singles.iter().skip(1).any(|(_, all)| !all);
            let mut branches = Vec::new();
            for (sq, _) in singles {
                let count = match &sq.kind {
                    SingleQueryKind::SinglePart(_) => 1,
                    SingleQueryKind::MultiPart(mp) => mp.parts.len() + 1,
                };
                let parts = next_part..next_part + count;
                next_part += count;
                let last = hir.parts.get(parts.end - 1).ok_or_else(|| {
                    CypherError::InvalidQuery("query parts do not match the parsed query".into())
                })?;
                let returns = matches!(last.operations.last(), Some(Operation::Return(_)));
                let outputs = if returns {
                    hir.arenas.scopes.get(last.output_scope).bindings.clone()
                } else {
                    Vec::new()
                };
                let columns = column_names(final_return(sq), &outputs, &hir, &cst);
                branches.push(Branch {
                    parts,
                    columns,
                    outputs,
                    returns,
                });
            }
            statements.push(Statement { branches, distinct });
        }
        if next_part != hir.parts.len() {
            return Err(CypherError::InvalidQuery(
                "query parts do not match the parsed query".into(),
            ));
        }

        let chained = chained_comparisons(&hir, text);
        let read_only = !hir.parts.iter().any(|p| p.operations.iter().any(mutates));
        Ok(Self {
            labels: names(&hir.arenas.labels),
            rel_types: names(&hir.arenas.relationship_types),
            keys: names(&hir.arenas.property_keys),
            parameters: names(&hir.arenas.parameters),
            functions,
            statements,
            chained,
            read_only,
            hir,
        })
    }

    /// Whether executing the query can change the graph.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// The names of the parameters the query uses (without `$`).
    pub fn parameters(&self) -> impl Iterator<Item = &str> {
        self.parameters.iter().map(String::as_str)
    }
}

impl std::str::FromStr for Query {
    type Err = CypherError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

/// Works around decypher 0.2.0-alpha.9 lowering comma-separated patterns
/// (`MATCH (a)-->(b), (c)-->(d)`) with relationship endpoints relative to
/// their own part instead of the whole pattern: shifts them by the part's
/// first node. Parts already lowered with global indices are left alone, so
/// this is a no-op once decypher is fixed.
///
/// The node count of every part comes from the AST, whose `MATCH` and
/// `CREATE` clauses are lowered in source order.
fn fix_pattern_offsets(ast: &decypher::ast::Query, hir: &mut HirQuery) -> Result<(), CypherError> {
    use decypher::ast::clause::ForeachUpdate;
    use decypher::ast::pattern::{Pattern, PatternElement};
    use decypher::ast::query::{ReadingClause, UpdatingClause};

    fn nodes(e: &PatternElement) -> usize {
        match e {
            PatternElement::Path { chains, .. } => chains.len() + 1,
            PatternElement::Parenthesized(inner) => nodes(inner),
            PatternElement::Quantified { element, .. } => nodes(element),
        }
    }
    fn part_sizes(p: &Pattern) -> Vec<usize> {
        p.parts
            .iter()
            .map(|part| nodes(&part.anonymous.element))
            .collect()
    }
    fn reading(out: &mut Vec<Vec<usize>>, rc: &ReadingClause) {
        if let ReadingClause::Match(m) = rc {
            out.push(part_sizes(&m.pattern));
        }
    }
    fn updating(out: &mut Vec<Vec<usize>>, uc: &UpdatingClause) {
        match uc {
            UpdatingClause::Create(c) => out.push(part_sizes(&c.pattern)),
            UpdatingClause::Foreach(f) => foreach(out, &f.updates),
            _ => {}
        }
    }
    fn foreach(out: &mut Vec<Vec<usize>>, updates: &[ForeachUpdate]) {
        for u in updates {
            match u {
                ForeachUpdate::Create(c) => out.push(part_sizes(&c.pattern)),
                ForeachUpdate::Foreach(f) => foreach(out, &f.updates),
                _ => {}
            }
        }
    }
    fn single(out: &mut Vec<Vec<usize>>, sq: &SingleQuery) {
        let (parts, last) = match &sq.kind {
            SingleQueryKind::SinglePart(sp) => (&[][..], sp),
            SingleQueryKind::MultiPart(mp) => (&mp.parts[..], &mp.final_part),
        };
        for part in parts {
            part.reading_clauses.iter().for_each(|rc| reading(out, rc));
            part.updating_clauses
                .iter()
                .for_each(|uc| updating(out, uc));
        }
        last.reading_clauses.iter().for_each(|rc| reading(out, rc));
        if let SinglePartBody::Updating { updating: ucs, .. } = &last.body {
            ucs.iter().for_each(|uc| updating(out, uc));
        }
    }

    let mut sizes = Vec::new();
    for body in &ast.statements {
        match body {
            QueryBody::SingleQuery(sq) => single(&mut sizes, sq),
            QueryBody::Regular(rq) => {
                single(&mut sizes, &rq.single_query);
                rq.unions
                    .iter()
                    .for_each(|u| single(&mut sizes, &u.single_query));
            }
            _ => {}
        }
    }

    fn patterns<'a>(ops: &'a mut [Operation], out: &mut Vec<&'a mut decypher::hir::GraphPattern>) {
        for op in ops {
            match op {
                Operation::Match(m) | Operation::OptionalMatch(m) => out.push(&mut m.pattern),
                Operation::Create(c) => out.push(&mut c.pattern),
                Operation::Foreach(f) => patterns(&mut f.operations, out),
                _ => {}
            }
        }
    }
    let mut targets = Vec::new();
    for part in &mut hir.parts {
        patterns(&mut part.operations, &mut targets);
    }
    let mismatch = || CypherError::InvalidQuery("pattern does not match the parsed query".into());
    if targets.len() != sizes.len() {
        return Err(mismatch());
    }
    for (pattern, sizes) in targets.into_iter().zip(sizes) {
        if sizes.iter().sum::<usize>() != pattern.nodes.len()
            || sizes.iter().map(|n| n - 1).sum::<usize>() != pattern.relationships.len()
        {
            return Err(mismatch());
        }
        let (mut node_start, mut rel_start) = (0, 0);
        for n in sizes {
            let rels = &mut pattern.relationships[rel_start..rel_start + n - 1];
            if node_start > 0 && rels.first().is_some_and(|r| r.left.min(r.right) == 0) {
                for r in rels {
                    r.left += node_start;
                    r.right += node_start;
                }
            }
            node_start += n;
            rel_start += n - 1;
        }
    }
    Ok(())
}

/// decypher 0.2.0-alpha.9 lowers `a < b < c` like `(a < b) < c`: a
/// comparison whose left operand is a comparison. The two differ only in the
/// source: a comparison's span covers its `op rhs` part, which a chain follows
/// directly with the next operator and a parenthesized operand with `)`.
fn chained_comparisons(hir: &HirQuery, text: &str) -> Vec<bool> {
    use decypher::hir::ExprKind;
    let exprs = &hir.arenas.expressions;
    exprs
        .iter()
        .map(|(_, e)| match &e.kind {
            ExprKind::Comparison { left, .. } => match &exprs.get(*left).kind {
                ExprKind::Comparison { .. } => {
                    let end = exprs.get(*left).span.end;
                    text.get(end..)
                        .and_then(|rest| rest.trim_start().chars().next())
                        .is_some_and(|c| c != ')')
                }
                _ => false,
            },
            _ => false,
        })
        .collect()
}

fn mutates(op: &Operation) -> bool {
    match op {
        Operation::Create(_)
        | Operation::Merge(_)
        | Operation::Set(_)
        | Operation::Remove(_)
        | Operation::Delete(_) => true,
        Operation::Foreach(f) => f.operations.iter().any(mutates),
        _ => false,
    }
}

/// Resolves every interned name once; ids are dense, starting at 0.
fn names(interner: &Interner<Id>) -> Vec<String> {
    (0..)
        .map_while(|i| interner.name_of(Id(i)).map(str::to_owned))
        .collect()
}

fn final_return(sq: &SingleQuery) -> Option<&Return> {
    let body = match &sq.kind {
        SingleQueryKind::SinglePart(sp) => &sp.body,
        SingleQueryKind::MultiPart(mp) => &mp.final_part.body,
    };
    match body {
        SinglePartBody::Return(r) => Some(r),
        SinglePartBody::Updating { return_clause, .. } => return_clause.as_ref(),
        _ => None,
    }
}

/// Column names: the binding names of variables passed through by `*`
/// (sorted by the lowering), then each item's alias, variable name, or
/// expression text exactly as written (`count(*)`, `n.name`).
fn column_names(
    ret: Option<&Return>,
    outputs: &[BindingId],
    hir: &HirQuery,
    cst: &decypher::syntax::SyntaxNode,
) -> Vec<String> {
    let items = ret.map_or(&[][..], |r| &r.items[..]);
    let star = outputs.len().saturating_sub(items.len());
    let texts = ret.map(|r| item_texts(cst, r)).unwrap_or_default();
    let mut columns: Vec<String> = outputs[..star]
        .iter()
        .map(|&b| hir.arenas.bindings.get(b).name.clone())
        .collect();
    columns.extend(items.iter().enumerate().map(|(i, item)| {
        match (&item.alias, &item.expression) {
            (Some(alias), _) => alias.name.name.clone(),
            (None, Expression::Variable(v)) => v.name.name.clone(),
            (None, expr) => texts.get(i).cloned().unwrap_or_else(|| expr.to_cypher()),
        }
    }));
    columns.truncate(outputs.len());
    columns
}

/// The source text of each projection item of `ret`, from the CST.
fn item_texts(cst: &decypher::syntax::SyntaxNode, ret: &Return) -> Vec<String> {
    use decypher::syntax::SyntaxKind;
    let clause = cst.descendants().find(|n| {
        let range = n.text_range();
        n.kind() == SyntaxKind::RETURN_CLAUSE
            && usize::from(range.start()) <= ret.span.start
            && ret.span.start < usize::from(range.end())
    });
    let Some(items) = clause.and_then(|c| {
        c.descendants()
            .find(|n| n.kind() == SyntaxKind::PROJECTION_ITEMS)
    }) else {
        return Vec::new();
    };
    items
        .children()
        .filter(|n| n.kind() == SyntaxKind::PROJECTION_ITEM)
        .map(|n| n.text().to_string().trim().to_owned())
        .collect()
}

impl Query {
    /// Runs the query against `graph`. Fails if the query would modify the
    /// graph; use [`Query::execute_mut`] for those.
    pub fn execute<G: crate::CypherGraph>(
        &self,
        graph: &G,
        params: &crate::Parameters,
    ) -> Result<crate::QueryResult, CypherError> {
        if !self.read_only {
            return Err(CypherError::InvalidQuery(
                "the query modifies the graph; use execute_mut / cypher_mut".into(),
            ));
        }
        crate::exec::Executor::run(self, crate::exec::ReadOnly(graph), params)
    }

    /// Runs the query against `graph`, applying its updates.
    pub fn execute_mut<G: crate::CypherGraphMut>(
        &self,
        graph: &mut G,
        params: &crate::Parameters,
    ) -> Result<crate::QueryResult, CypherError> {
        crate::exec::Executor::run(self, crate::exec::ReadWrite(graph), params)
    }
}
