//! Data-flow complexity report for `UniPack` (Dunsmore-style approximation).
//!
//! This integration test tracks variable definitions and uses to estimate
//! data-flow complexity, then prints report sections used by
//! `dev/scripts/complexity_report.sh`.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
struct FunctionDataFlow {
    name: String,
    file: PathBuf,
    complexity: u32,
    definitions: u32,
    uses: u32,
}

fn rust_files_under(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().and_then(|n| n.to_str()) == Some("target") {
                    continue;
                }
                files.extend(rust_files_under(&path));
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
    files
}

fn track_pat_definitions(pat: &syn::Pat, defs: &mut HashSet<String>) {
    match pat {
        syn::Pat::Ident(p) => {
            defs.insert(p.ident.to_string());
        }
        syn::Pat::Tuple(p) => {
            for elem in &p.elems {
                track_pat_definitions(elem, defs);
            }
        }
        syn::Pat::Struct(p) => {
            for field in &p.fields {
                track_pat_definitions(&field.pat, defs);
            }
        }
        syn::Pat::Slice(p) => {
            for elem in &p.elems {
                track_pat_definitions(elem, defs);
            }
        }
        syn::Pat::Or(p) => {
            for case in &p.cases {
                track_pat_definitions(case, defs);
            }
        }
        _ => {}
    }
}

fn track_expr_uses(expr: &syn::Expr, uses: &mut HashSet<String>) {
    match expr {
        syn::Expr::Path(p) => {
            if let Some(ident) = p.path.get_ident() {
                uses.insert(ident.to_string());
            }
        }
        syn::Expr::Assign(a) => {
            if let syn::Expr::Path(path) = &*a.left
                && let Some(ident) = path.path.get_ident()
            {
                uses.insert(ident.to_string());
            }
            track_expr_uses(&a.right, uses);
        }
        syn::Expr::Call(c) => {
            track_expr_uses(&c.func, uses);
            for arg in &c.args {
                track_expr_uses(arg, uses);
            }
        }
        syn::Expr::MethodCall(c) => {
            track_expr_uses(&c.receiver, uses);
            for arg in &c.args {
                track_expr_uses(arg, uses);
            }
        }
        syn::Expr::If(v) => {
            track_expr_uses(&v.cond, uses);
            for stmt in &v.then_branch.stmts {
                track_stmt_uses(stmt, uses);
            }
            if let Some((_, else_expr)) = &v.else_branch {
                track_expr_uses(else_expr, uses);
            }
        }
        syn::Expr::While(v) => {
            track_expr_uses(&v.cond, uses);
            for stmt in &v.body.stmts {
                track_stmt_uses(stmt, uses);
            }
        }
        syn::Expr::ForLoop(v) => {
            track_expr_uses(&v.expr, uses);
            for stmt in &v.body.stmts {
                track_stmt_uses(stmt, uses);
            }
        }
        syn::Expr::Loop(v) => {
            for stmt in &v.body.stmts {
                track_stmt_uses(stmt, uses);
            }
        }
        syn::Expr::Match(v) => {
            track_expr_uses(&v.expr, uses);
            for arm in &v.arms {
                if let Some((_, g)) = &arm.guard {
                    track_expr_uses(g, uses);
                }
                track_expr_uses(&arm.body, uses);
            }
        }
        syn::Expr::Block(v) => {
            for stmt in &v.block.stmts {
                track_stmt_uses(stmt, uses);
            }
        }
        syn::Expr::Binary(v) => {
            track_expr_uses(&v.left, uses);
            track_expr_uses(&v.right, uses);
        }
        syn::Expr::Unary(v) => track_expr_uses(&v.expr, uses),
        syn::Expr::Try(v) => track_expr_uses(&v.expr, uses),
        _ => {}
    }
}

fn track_stmt_uses(stmt: &syn::Stmt, uses: &mut HashSet<String>) {
    match stmt {
        syn::Stmt::Expr(expr, _) => track_expr_uses(expr, uses),
        syn::Stmt::Local(local) => {
            if let Some(init) = &local.init {
                track_expr_uses(&init.expr, uses);
            }
        }
        syn::Stmt::Item(_) | syn::Stmt::Macro(_) => {}
    }
}

fn analyze_file(path: &Path) -> Result<Vec<FunctionDataFlow>, Box<dyn std::error::Error>> {
    let src = fs::read_to_string(path)?;
    let file = syn::parse_file(&src)?;
    let mut out = Vec::new();

    for item in file.items {
        match item {
            syn::Item::Fn(f) => {
                let mut defs = HashSet::new();
                let mut uses = HashSet::new();
                for arg in &f.sig.inputs {
                    if let syn::FnArg::Typed(typed) = arg {
                        track_pat_definitions(&typed.pat, &mut defs);
                    }
                }
                for stmt in &f.block.stmts {
                    if let syn::Stmt::Local(local) = stmt {
                        track_pat_definitions(&local.pat, &mut defs);
                    }
                    track_stmt_uses(stmt, &mut uses);
                }

                let du_pairs = defs.intersection(&uses).count();
                #[allow(clippy::cast_possible_truncation)]
                let complexity = du_pairs as u32;
                #[allow(clippy::cast_possible_truncation)]
                out.push(FunctionDataFlow {
                    name: f.sig.ident.to_string(),
                    file: path.to_path_buf(),
                    complexity,
                    definitions: defs.len() as u32,
                    uses: uses.len() as u32,
                });
            }
            syn::Item::Impl(imp) => {
                for impl_item in imp.items {
                    if let syn::ImplItem::Fn(m) = impl_item {
                        let mut defs = HashSet::new();
                        let mut uses = HashSet::new();
                        for arg in &m.sig.inputs {
                            match arg {
                                syn::FnArg::Receiver(_) => {
                                    defs.insert(String::from("self"));
                                }
                                syn::FnArg::Typed(typed) => {
                                    track_pat_definitions(&typed.pat, &mut defs);
                                }
                            }
                        }
                        for stmt in &m.block.stmts {
                            if let syn::Stmt::Local(local) = stmt {
                                track_pat_definitions(&local.pat, &mut defs);
                            }
                            track_stmt_uses(stmt, &mut uses);
                        }
                        let du_pairs = defs.intersection(&uses).count();
                        #[allow(clippy::cast_possible_truncation)]
                        let complexity = du_pairs as u32;
                        #[allow(clippy::cast_possible_truncation)]
                        out.push(FunctionDataFlow {
                            name: m.sig.ident.to_string(),
                            file: path.to_path_buf(),
                            complexity,
                            definitions: defs.len() as u32,
                            uses: uses.len() as u32,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    Ok(out)
}

#[test]
fn test_data_flow_complexity() {
    let mut files = rust_files_under(Path::new("src"));
    files.extend(rust_files_under(Path::new("tests")));

    let mut functions = Vec::new();
    for file in files {
        if let Ok(mut local) = analyze_file(&file) {
            functions.append(&mut local);
        }
    }

    assert!(
        !functions.is_empty(),
        "No Rust functions found for data-flow analysis"
    );

    functions.sort_by_key(|f| std::cmp::Reverse(f.complexity));
    let total_complexity: u32 = functions.iter().map(|f| f.complexity).sum();
    let total_defs: u32 = functions.iter().map(|f| f.definitions).sum();
    let total_uses: u32 = functions.iter().map(|f| f.uses).sum();

    println!("\n=== Data Flow Complexity Report (Dunsmore) ===");
    println!(
        "Total files analyzed: {}",
        rust_files_under(Path::new("src")).len()
    );
    println!("Total functions/methods: {}", functions.len());
    println!("Total project complexity: {total_complexity}");
    println!("Total variable definitions: {total_defs}");
    println!("Total variable uses: {total_uses}");
    #[allow(clippy::cast_precision_loss)]
    let avg = f64::from(total_complexity) / functions.len() as f64;
    println!("Average complexity per function: {avg:.2}");

    println!("\n=== Top 10 Most Complex Functions ===");
    for (idx, f) in functions.iter().take(10).enumerate() {
        println!(
            "{}. {} (complexity: {}, defs: {}, uses: {}) - {}:0",
            idx + 1,
            f.name,
            f.complexity,
            f.definitions,
            f.uses,
            f.file.display()
        );
    }
}
