# rustdoc-twoslash

A fork of [rust-lang/rust](https://github.com/rust-lang/rust)'s rustdoc with **twoslash-style type hover annotations** on code blocks.

Hover over any identifier in a ```` ```rust ```` doc example to see its inferred type — the same information your IDE shows on hover, rendered directly in the documentation.

## How it works

```
Doc comment ````rust code blocks
        │
        ▼
   pulldown-cmark (markdown.rs)
        │
        ▼
   CodeBlocks iterator extracts code text
        │
        ▼
   twoslash.rs ──► twoslash-rust ──► rust-analyzer
        │              │                  │
        │              │    scaffolds a temp Cargo project,
        │              │    runs rust-analyzer, returns
        │              │    type info per identifier
        │              ◄──────────────────┘
        ▼
   highlight.rs merges annotations into DecorationInfo
        │
        ▼
   push_token() emits <span data-type="..."> attributes
        │
        ▼
   rustdoc.css renders hover popovers via CSS + JS
```

## Changed files

This repo contains **only the modified `src/librustdoc/` files** from the rust-lang/rust fork. Apply the included `twoslash.patch` to a fresh checkout of rust-lang/rust to reproduce:

| File | Change |
|------|--------|
| `src/librustdoc/Cargo.toml` | Added `twoslash_rust` and `once_cell` dependencies |
| `src/librustdoc/html/twoslash.rs` | **New** — integration module: `process_code_block()`, `is_enabled()`, `needs_main_wrapper()` with offset adjustment |
| `src/librustdoc/html/highlight.rs` | Extended `DecorationInfo` with `type_annotations`, fuzzy token matching, `data-type` attribute emission |
| `src/librustdoc/html/highlight/tests.rs` | Updated tests for new `DecorationInfo` field |
| `src/librustdoc/html/markdown.rs` | Modified `CodeBlocks::next()` to call twoslash when `RUSTDOC_TWOSLASH=1` |
| `src/librustdoc/html/mod.rs` | Added `mod twoslash` |
| `src/librustdoc/html/render/mod.rs` | Minor update for annotation plumbing |
| `src/librustdoc/html/static/css/rustdoc.css` | Hover popover styles for `[data-type]` elements |
| `src/librustdoc/html/static/js/main.js` | Interactive popover positioning |

## Usage

```bash
# 1. Clone rust-lang/rust and apply the patch
git clone --depth 1 https://github.com/rust-lang/rust.git rustdoc-twoslash
cd rustdoc-twoslash
git apply /path/to/twoslash.patch

# 2. Build rustdoc
./x.py build library/std src/librustdoc

# 3. Run on your crate with twoslash enabled
cd /path/to/your-crate
RUSTDOC_TWOSLASH=1 \
  RUSTDOC=.../build/host/stage1/bin/rustdoc \
  RUSTC=.../build/host/stage1/bin/rustc \
  cargo doc --no-deps
```

## Dependencies

- [twoslash-rust](https://github.com/wevm/vocs/tree/next/twoslash-rust) — extracts type information from Rust code via rust-analyzer
- [rust-lang/rust](https://github.com/rust-lang/rust) — the base rustdoc being modified

## Demo

See [tmm/twoslash-demo](https://github.com/tmm/twoslash-demo) for a sample crate with generated docs.
