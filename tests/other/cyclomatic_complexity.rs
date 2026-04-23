//! Cyclomatic complexity report for `UniPack`.
//!
//! This integration test parses Rust source files and prints a complexity
//! ranking that can be consumed by `dev/scripts/complexity_report.sh`.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
struct FunctionComplexity {
    name: String,
    file: PathBuf,
    complexity: u32,
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

fn collect_expr_complexity(expr: &syn::Expr, score: &mut u32) {
    match expr {
        syn::Expr::If(v) => {
            *score += 1;
            collect_expr_complexity(&v.cond, score);
            for stmt in &v.then_branch.stmts {
                collect_stmt_complexity(stmt, score);
            }
            if let Some((_, else_expr)) = &v.else_branch {
                collect_expr_complexity(else_expr, score);
            }
        }
        syn::Expr::While(v) => {
            *score += 1;
            collect_expr_complexity(&v.cond, score);
            for stmt in &v.body.stmts {
                collect_stmt_complexity(stmt, score);
            }
        }
        syn::Expr::ForLoop(v) => {
            *score += 1;
            collect_expr_complexity(&v.expr, score);
            for stmt in &v.body.stmts {
                collect_stmt_complexity(stmt, score);
            }
        }
        syn::Expr::Loop(v) => {
            *score += 1;
            for stmt in &v.body.stmts {
                collect_stmt_complexity(stmt, score);
            }
        }
        syn::Expr::Match(v) => {
            *score += u32::try_from(v.arms.len()).unwrap_or(u32::MAX);
            for arm in &v.arms {
                if arm.guard.is_some() {
                    *score += 1;
                }
                collect_expr_complexity(&arm.body, score);
            }
        }
        syn::Expr::Binary(v) => {
            if matches!(v.op, syn::BinOp::And(_) | syn::BinOp::Or(_)) {
                *score += 1;
            }
            collect_expr_complexity(&v.left, score);
            collect_expr_complexity(&v.right, score);
        }
        syn::Expr::Try(v) => {
            *score += 1;
            collect_expr_complexity(&v.expr, score);
        }
        syn::Expr::Block(v) => {
            for stmt in &v.block.stmts {
                collect_stmt_complexity(stmt, score);
            }
        }
        syn::Expr::Call(v) => {
            collect_expr_complexity(&v.func, score);
            for arg in &v.args {
                collect_expr_complexity(arg, score);
            }
        }
        syn::Expr::MethodCall(v) => {
            collect_expr_complexity(&v.receiver, score);
            for arg in &v.args {
                collect_expr_complexity(arg, score);
            }
        }
        _ => {}
    }
}

fn collect_stmt_complexity(stmt: &syn::Stmt, score: &mut u32) {
    match stmt {
        syn::Stmt::Expr(expr, _) => collect_expr_complexity(expr, score),
        syn::Stmt::Local(local) => {
            if let Some(init) = &local.init {
                collect_expr_complexity(&init.expr, score);
            }
        }
        syn::Stmt::Item(_) | syn::Stmt::Macro(_) => {}
    }
}

fn analyze_file(path: &Path) -> Result<Vec<FunctionComplexity>, Box<dyn std::error::Error>> {
    let src = fs::read_to_string(path)?;
    let file = syn::parse_file(&src)?;
    let mut out = Vec::new();

    for item in file.items {
        match item {
            syn::Item::Fn(f) => {
                let mut complexity = 1;
                for stmt in &f.block.stmts {
                    collect_stmt_complexity(stmt, &mut complexity);
                }
                out.push(FunctionComplexity {
                    name: f.sig.ident.to_string(),
                    file: path.to_path_buf(),
                    complexity,
                });
            }
            syn::Item::Impl(imp) => {
                for impl_item in imp.items {
                    if let syn::ImplItem::Fn(m) = impl_item {
                        let mut complexity = 1;
                        for stmt in &m.block.stmts {
                            collect_stmt_complexity(stmt, &mut complexity);
                        }
                        out.push(FunctionComplexity {
                            name: m.sig.ident.to_string(),
                            file: path.to_path_buf(),
                            complexity,
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
fn test_cyclomatic_complexity() {
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
        "No Rust functions found for complexity analysis"
    );

    functions.sort_by_key(|f| std::cmp::Reverse(f.complexity));
    let total_complexity: u32 = functions.iter().map(|f| f.complexity).sum();

    println!("\n=== Cyclomatic Complexity Report ===");
    println!(
        "Total files analyzed: {}",
        rust_files_under(Path::new("src")).len()
    );
    println!("Total functions/methods: {}", functions.len());
    println!("Total project complexity: {total_complexity}");
    #[allow(clippy::cast_precision_loss)]
    let avg = f64::from(total_complexity) / functions.len() as f64;
    println!("Average complexity per function: {avg:.2}");

    println!("\n=== Top 10 Most Complex Functions ===");
    for (idx, f) in functions.iter().take(10).enumerate() {
        println!(
            "{}. {} (complexity: {}) - {}:0",
            idx + 1,
            f.name,
            f.complexity,
            f.file.display()
        );
    }
}
