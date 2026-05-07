# 7. Deterministic AST extraction for code via Tree-sitter; LLM extraction only for prose

Date: 2026-05-06

## Status

Accepted

## Context

LLM-based extraction is the obvious approach: feed the model a chunk, prompt for triples. It works for arbitrary text but has three problems:

- **Quality floor**: even strong models hallucinate relationships, especially on long contexts.
- **Cost**: every changed file becomes a model call.
- **Non-determinism**: re-running on the same input yields different triples.

For code, none of these are necessary. Source files have a parser. Function calls, imports, class definitions, type references — all of these are exactly recoverable via AST. Tree-sitter ships grammars for >40 languages, runs in milliseconds, and never hallucinates.

The book's "multimodal extraction" framing [ch.90] (Tree-sitter for code, OCR for PDFs, VLMs for images, Whisper for audio) reinforces this: pick the right tool per modality.

## Decision

Two extraction pipelines, two retention policies:

### Code pipeline (deterministic)

- **Trigger**: on file write/edit, on git commit hook, on initial repo scan.
- **Tool**: native Tree-sitter bindings in the implementation language. The Rust core (ADR-0013) uses the `tree-sitter` crate plus per-language bundled grammar crates (`tree-sitter-python`, `-typescript`, `-go`, `-rust`). Python bindings are acceptable only for prototyping and must not appear in the shipped binary.
- **Triples emitted**:
  - `File → defines → Symbol`
  - `Symbol → calls → Symbol`
  - `Module → imports → Module`
  - `Class → extends → Class`
  - `Function → has_test → TestFunction`
- **Provenance**: `EXTRACTED`, **confidence 0.90** (per ADR-0003).
- **Lifecycle**: code edges are invalidated when the file changes; re-extracted on next write.

### Prose pipeline (LLM)

- **Trigger**: chat messages, commit messages, PR descriptions, markdown, docstrings, comments.
- **Tool**: structured-output LLM call with the project ontology as schema.
- **Triples emitted**: preferences, decisions, bugs, ownership, constraints — semantic claims that have no AST analog.
- **Provenance**: `INFERRED` or `EXTRACTED` (depending on corroboration), per ADR-0003 confidence policy.

### Routing

A pre-extractor classifier picks pipeline by content-type:

- file extension in supported-grammar set → code pipeline,
- `.md`, `.txt`, chat content, commit messages → prose pipeline,
- PDFs and images → existing Docling/OCR path (ADR-0002 vector store) plus prose pipeline on the extracted text.

## Consequences

- (+) Code memory is exact, free, and regenerable. No model in the hot path for code.
- (+) Prose pipeline cost is bounded — only runs on natural language.
- (+) Code edges have a clean invalidation signal (file change), unlike LLM-extracted claims.
- (−) Two pipelines to maintain. Schema drift between them is a real risk; both must emit triples with the same predicate vocabulary.
- (−) Tree-sitter language coverage is broad but not universal — exotic or new languages fall back to LLM extraction.
- (−) Boilerplate per language: each Tree-sitter grammar needs query patterns mapping AST nodes → predicates. Mitigated by starting with Python, TypeScript, Go, Rust and adding others on demand.
