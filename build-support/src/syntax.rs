//! Check handwritten Rust syntax, including native macro bodies, before compiling the application.
use proc_macro2::{Delimiter, TokenStream, TokenTree};
use syn::{
    Expr, ItemFn, Stmt,
    visit::{self, Visit},
};

/// Return source diagnostics without compiling or executing the inspected file.
pub fn violations(source: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let tokens = match source.parse::<TokenStream>() {
        Ok(tokens) => tokens,
        Err(error) => return vec![format!("cannot tokenize source: {error}")],
    };
    let lines: Vec<_> = source.lines().collect();
    inspect(tokens, &lines, &mut errors);
    errors
}

/// Inspect functions in each lexical group; callbacks inside define_class! remain visible here.
fn inspect(tokens: TokenStream, lines: &[&str], errors: &mut Vec<String>) {
    let tokens: Vec<_> = tokens.into_iter().collect();
    let mut previous_end = None;
    for (index, token) in tokens.iter().enumerate() {
        // A function pointer type has no following name and must not be treated as a declaration.
        if is_function(&tokens, index)
            && let Some(function) = function_at(&tokens, index)
        {
            let line = function.sig.fn_token.span.start().line - 1;
            let head = metadata_start(lines, line);
            // Keep purpose comments attached to their function, with a blank line between siblings.
            if let Some(end) = previous_end
                && !lines
                    .get(end..head)
                    .is_some_and(|between| between.iter().any(|line| line.trim().is_empty()))
            {
                errors.push(format!(
                    "line {}: leave a blank line between functions",
                    head + 1
                ));
            }
            // Metadata alone does not explain why an operation exists.
            if !lines[head..=line]
                .iter()
                .any(|line| line.trim_start().starts_with("//"))
            {
                errors.push(format!(
                    "line {}: add a short purpose comment above the function",
                    line + 1
                ));
            }
            previous_end = Some(function.block.brace_token.span.close().end().line);
            check_wrapper(&function, errors);
            Guards { errors }.visit_block(&function.block);
        }
        // Tokens in native and CEF macros contain ordinary Rust function bodies too.
        if let TokenTree::Group(group) = token {
            inspect(group.stream(), lines, errors);
        }
    }
}

/// Identify named declarations rather than bare fn types in signatures and action tables.
fn is_function(tokens: &[TokenTree], index: usize) -> bool {
    matches!(&tokens[index], TokenTree::Ident(ident) if ident == "fn")
        && matches!(tokens.get(index + 1), Some(TokenTree::Ident(_)))
}

/// Parse a single named function while leaving enclosing macro syntax to its own owner.
fn function_at(tokens: &[TokenTree], start: usize) -> Option<ItemFn> {
    let end = (start + 2..tokens.len()).find(|index| match &tokens[*index] {
        TokenTree::Group(group) => group.delimiter() == Delimiter::Brace,
        TokenTree::Punct(mark) => mark.as_char() == ';',
        _ => false,
    })?;
    syn::parse2(tokens[start..=end].iter().cloned().collect()).ok()
}

/// Find attached comments and attributes, so a blank line belongs before the whole declaration.
fn metadata_start(lines: &[&str], line: usize) -> usize {
    let mut start = line;
    let mut brackets = 0_isize;
    while start > 0 {
        let previous = lines[start - 1].trim();
        // A blank line or normal Rust statement separates metadata from the preceding item.
        if previous.is_empty() {
            break;
        }
        // Attribute arguments may span several lines; retain the entire metadata block.
        if previous.starts_with("//")
            || previous.starts_with("#[")
            || brackets > 0
            || previous.ends_with(']')
        {
            brackets +=
                previous.matches(']').count() as isize - previous.matches('[').count() as isize;
            start -= 1;
            continue;
        }
        break;
    }
    start
}

/// A substantial final branch in a unit function can use a guard and leave its work unindented.
fn check_wrapper(function: &ItemFn, errors: &mut Vec<String>) {
    // Value-producing functions need their expression, rather than an inserted unit return.
    if !matches!(function.sig.output, syn::ReturnType::Default) {
        return;
    }
    // Only a final branch can be inverted without changing the following statements' execution.
    let Some(Stmt::Expr(Expr::If(branch), _)) = function.block.stmts.last() else {
        return;
    };
    // Tiny optional updates remain compact; alternative branches may each perform required work.
    if branch.else_branch.is_some() || branch.then_branch.stmts.len() < 3 {
        return;
    }
    errors.push(format!(
        "line {}: use an early guard instead of wrapping the remaining function body",
        branch.if_token.span.start().line
    ));
}

struct Guards<'a> {
    errors: &'a mut Vec<String>,
}

impl<'ast> Visit<'ast> for Guards<'_> {
    /// Catch an else after a terminating guard; expression-valued choices remain valid Rust.
    fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
        // The opposite branch can be unindented once the guard has already left this scope.
        if node.else_branch.is_some() && terminates(&node.then_branch) {
            self.errors.push(format!(
                "line {}: return early and remove the redundant else",
                node.if_token.span.start().line
            ));
        }
        visit::visit_expr_if(self, node);
    }
}

/// Recognize explicit scope exits without guessing about calls that may or may not return.
fn terminates(block: &syn::Block) -> bool {
    matches!(
        block.stmts.last(),
        Some(Stmt::Expr(
            Expr::Return(_) | Expr::Break(_) | Expr::Continue(_),
            _
        ))
    )
}

#[cfg(test)]
#[path = "../tests/source_checks.rs"]
mod tests;
