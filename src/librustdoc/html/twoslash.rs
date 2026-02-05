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
    let cargo_path = std::env::var("RUSTDOC_TWOSLASH_CARGO_TOML")
        .ok()
        .unwrap_or_else(|| "Cargo.toml".to_string());

    let content = match std::fs::read_to_string(&cargo_path) {
        Ok(c) => c,
        Err(_) => {
            eprintln!("twoslash: no Cargo.toml found, external deps won't have annotations");
            return None;
        }
    };

    eprintln!("twoslash: using Cargo.toml from {}", cargo_path);

    // Add the crate being documented as a path dependency so that
    // code examples referencing `crate_name::foo` resolve correctly.
    let crate_dir = std::env::current_dir().ok()?;
    let augmented = inject_self_dependency(&content, &crate_dir.to_string_lossy());
    Some(augmented)
}

/// Inject the crate being documented as a path dependency.
///
/// Parses the crate name from the Cargo.toml and adds it as:
///   `crate_name = { path = "/path/to/crate" }`
fn inject_self_dependency(cargo_toml: &str, crate_path: &str) -> String {
    let crate_name = cargo_toml
        .lines()
        .find(|l| l.trim().starts_with("name"))
        .and_then(|l| {
            let val = l.split('=').nth(1)?.trim();
            Some(val.trim_matches('"').to_string())
        });

    let Some(name) = crate_name else {
        return cargo_toml.to_string();
    };

    // Use underscore form for the dependency key (Cargo normalizes hyphens)
    let dep_key = name.replace('-', "_");
    let dep_line = format!("{} = {{ path = \"{}\" }}", dep_key, crate_path);

    // Check if [dependencies] section exists
    if let Some(pos) = cargo_toml.find("[dependencies]") {
        let after = pos + "[dependencies]".len();
        eprintln!("twoslash: adding self-dependency: {}", dep_line);
        format!("{}\n{}{}", &cargo_toml[..after], dep_line, &cargo_toml[after..])
    } else {
        eprintln!("twoslash: adding self-dependency: {}", dep_line);
        format!("{}\n[dependencies]\n{}\n", cargo_toml, dep_line)
    }
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

/// Item-level keyword prefixes that indicate top-level declarations
const ITEM_KEYWORDS: &[&str] = &[
    "fn ", "struct ", "enum ", "impl ", "trait ", "mod ",
    "pub ", "extern ", "const ", "static ", "type ",
    "use ", "#[", "#!",
];

/// Check if a line starts a top-level item
fn is_item_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    ITEM_KEYWORDS.iter().any(|kw| trimmed.starts_with(kw))
}

/// Split code into preamble (top-level items) and body (statements).
///
/// Handles mixed code like:
/// ```text
/// fn helper() -> i32 { 42 }     // preamble
///                                // preamble (blank line)
/// let x = helper();              // body (needs fn main wrapper)
/// ```
///
/// Returns (preamble, body) where preamble contains complete item definitions
/// and body contains statement-level code that needs fn main() wrapping.
/// If no wrapping is needed, body is empty.
fn split_items_and_statements(code: &str) -> (String, String) {
    // If code already has fn main, no splitting needed
    if code.contains("fn main") {
        return (code.to_string(), String::new());
    }

    let lines: Vec<&str> = code.lines().collect();
    let mut brace_depth: i32 = 0;
    let mut split_point = 0; // byte offset where body starts
    let mut line_byte_offset = 0;

    for (_i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        if brace_depth > 0 {
            // Inside a braced item, track depth
            brace_depth += trimmed.chars().filter(|&c| c == '{').count() as i32;
            brace_depth -= trimmed.chars().filter(|&c| c == '}').count() as i32;
            line_byte_offset += line.len() + 1; // +1 for newline
            if brace_depth <= 0 {
                brace_depth = 0;
                split_point = line_byte_offset;
            }
            continue;
        }

        if trimmed.is_empty() {
            // Blank lines between items are part of preamble
            line_byte_offset += line.len() + 1;
            split_point = line_byte_offset;
            continue;
        }

        if is_item_line(trimmed) {
            // Count braces on this line
            brace_depth += trimmed.chars().filter(|&c| c == '{').count() as i32;
            brace_depth -= trimmed.chars().filter(|&c| c == '}').count() as i32;
            if brace_depth < 0 {
                brace_depth = 0;
            }
            line_byte_offset += line.len() + 1;
            if brace_depth == 0 {
                split_point = line_byte_offset;
            }
            continue;
        }

        // This line is a statement, everything from here is body
        break;
    }

    let split_point = split_point.min(code.len());
    let preamble = &code[..split_point];
    let body = &code[split_point..];

    if body.trim().is_empty() {
        // All items, no wrapping needed
        (code.to_string(), String::new())
    } else {
        (preamble.to_string(), body.to_string())
    }
}

/// Process a code block and extract type annotations
pub fn process_code_block(code: &str) -> Vec<TypeAnnotation> {
    let mut analyzer = match ANALYZER.lock() {
        Ok(a) => a,
        Err(_) => return vec![],
    };

    // Split into preamble (top-level items) and body (statements needing fn main)
    let (preamble, body) = split_items_and_statements(code);
    let needs_wrap = !body.is_empty();
    let (wrapped_code, preamble_len, fn_main_offset) = if needs_wrap {
        let fn_main_prefix = "fn main() {\n";
        let suffix = "\n}";
        let wrapped = format!("{}{}{}{}", preamble, fn_main_prefix, body, suffix);
        (wrapped, preamble.len() as u32, fn_main_prefix.len() as u32)
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
                    // - 0..preamble_len: preamble items (same positions in original)
                    // - preamble_len..(preamble_len+fn_main_offset): "fn main() {\n" (skip)
                    // - (preamble_len+fn_main_offset)..end: body code (subtract fn_main_offset)
                    let wrapper_start = preamble_len + fn_main_offset;
                    
                    let adjusted_start = if fn_main_offset == 0 {
                        // No wrapping, use as-is
                        info.start
                    } else if info.start < preamble_len {
                        // In preamble section, position is same in original
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
