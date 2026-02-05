//! Twoslash integration for rustdoc
//!
//! Provides type hover information for code blocks.
//!
//! When enabled via RUSTDOC_TWOSLASH=1, this module processes Rust code blocks
//! through rust-analyzer to extract type information for hover annotations.

use once_cell::sync::Lazy;
use std::sync::Mutex;
use twoslash_rust::{Analyzer, AnalyzerSettings};

/// Global analyzer instance (reused across code blocks)
static ANALYZER: Lazy<Mutex<Analyzer>> = Lazy::new(|| {
    let cargo_toml = resolve_cargo_toml();
    Mutex::new(Analyzer::new(AnalyzerSettings {
        cargo_toml,
        target_dir: Some("/tmp/rustdoc-twoslash-cache".into()),
    }))
});

/// Resolve the Cargo.toml to use for twoslash analysis.
///
/// Checks RUSTDOC_TWOSLASH_CARGO_TOML env var first, then tries to find
/// Cargo.toml relative to the current directory. This lets twoslash-rust
/// scaffold temp projects with the same dependencies as the crate being documented.
fn resolve_cargo_toml() -> Option<String> {
    if let Ok(path) = std::env::var("RUSTDOC_TWOSLASH_CARGO_TOML") {
        if let Ok(content) = std::fs::read_to_string(&path) {
            eprintln!("twoslash: using Cargo.toml from {}", path);
            return Some(content);
        }
    }
    if let Ok(content) = std::fs::read_to_string("Cargo.toml") {
        eprintln!("twoslash: using Cargo.toml from current directory");
        return Some(content);
    }
    eprintln!("twoslash: no Cargo.toml found, external deps won't have annotations");
    None
}

/// Information about a type annotation to render
#[derive(Debug, Clone)]
pub struct TypeAnnotation {
    /// Byte offset in the code where the token starts
    pub start: u32,
    /// Length of the token in bytes
    pub length: u32,
    /// The type information to display on hover
    pub type_text: String,
    /// Optional documentation
    pub docs: Option<String>,
}

/// Check if twoslash processing is enabled
pub fn is_enabled() -> bool {
    std::env::var("RUSTDOC_TWOSLASH").is_ok()
}

/// Check if code needs to be wrapped in fn main() for analysis
/// Returns (needs_wrap, use_statements) where use_statements should be placed before fn main
fn needs_main_wrapper(code: &str) -> (bool, String) {
    // If it already has fn main, don't wrap
    if code.contains("fn main") {
        return (false, String::new());
    }
    
    let trimmed = code.trim_start();
    
    // If code starts with item-level keywords (not use), don't wrap
    let item_keywords = [
        "fn ", "struct ", "enum ", "impl ", "trait ", "mod ", 
        "pub ", "extern ", "const ", "static ", "type ",
        "#[", "#!", 
    ];
    
    for kw in item_keywords {
        if trimmed.starts_with(kw) {
            return (false, String::new());
        }
    }
    
    // Handle 'use' statements specially: extract them and wrap the rest
    if trimmed.starts_with("use ") {
        // Collect all use statements
        let mut use_stmts = String::new();
        let mut rest_start = 0;
        
        for line in code.lines() {
            let line_trimmed = line.trim();
            if line_trimmed.starts_with("use ") || line_trimmed.is_empty() {
                use_stmts.push_str(line);
                use_stmts.push('\n');
                rest_start += line.len() + 1; // +1 for newline
            } else {
                break;
            }
        }
        
        // If there's more code after use statements, we need to wrap it
        let rest = &code[rest_start.min(code.len())..];
        if !rest.trim().is_empty() {
            return (true, use_stmts);
        }
        // Only use statements, no wrapping needed
        return (false, String::new());
    }
    
    // Otherwise, assume it's statement-level code that needs wrapping
    (true, String::new())
}

/// Process a code block and extract type annotations
pub fn process_code_block(code: &str) -> Vec<TypeAnnotation> {
    let mut analyzer = match ANALYZER.lock() {
        Ok(a) => a,
        Err(_) => return vec![],
    };

    // Wrap code in fn main() if needed for analysis
    let (needs_wrap, use_stmts) = needs_main_wrapper(code);
    // offset_for_fn_main is the extra bytes added by "fn main() {\n" that we need to subtract
    // from positions in the wrapped code to get positions in the original code
    let (wrapped_code, use_stmts_len, fn_main_offset) = if needs_wrap {
        // Build wrapped code: use statements + fn main { rest of code }
        let use_len = use_stmts.len();
        let rest_of_code = if use_len > 0 {
            &code[use_len..]
        } else {
            code
        };
        let fn_main_prefix = "fn main() {\n";
        let suffix = "\n}";
        let wrapped = format!("{}{}{}{}", use_stmts, fn_main_prefix, rest_of_code, suffix);
        (wrapped, use_len as u32, fn_main_prefix.len() as u32)
    } else {
        (code.to_string(), 0, 0)
    };

    match analyzer.analyze(&wrapped_code) {
        Ok(result) => {
            result
                .static_quick_infos
                .into_iter()
                .filter_map(|info| {
                    // Adjust offsets for wrapped code
                    // Positions in the wrapped code:
                    // - 0..use_stmts_len: use statements (same positions in original)
                    // - use_stmts_len..(use_stmts_len+fn_main_offset): "fn main() {\n" (skip)
                    // - (use_stmts_len+fn_main_offset)..end: rest of code (subtract fn_main_offset)
                    let wrapper_start = use_stmts_len + fn_main_offset;
                    
                    let adjusted_start = if fn_main_offset == 0 {
                        // No wrapping, use as-is
                        info.start
                    } else if info.start < use_stmts_len {
                        // In use statements section, position is same in original
                        info.start
                    } else if info.start < wrapper_start {
                        // In "fn main() {\n" part, skip this annotation
                        return None;
                    } else {
                        // In the body of fn main, subtract fn_main_offset
                        info.start - fn_main_offset
                    };
                    
                    // Skip annotations that extend past the original code
                    let original_len = code.len() as u32;
                    if adjusted_start >= original_len {
                        return None;
                    }
                    
                    // Skip annotations for single-character tokens that are likely operators/punctuation
                    if info.length <= 1 {
                        return None;
                    }
                    
                    Some(TypeAnnotation {
                        start: adjusted_start,
                        length: info.length,
                        type_text: info.text,
                        docs: info.docs,
                    })
                })
                .collect()
        }
        Err(e) => {
            eprintln!("twoslash error: {}", e);
            vec![]
        }
    }
}
