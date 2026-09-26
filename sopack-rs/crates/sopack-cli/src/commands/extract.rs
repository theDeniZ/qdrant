//! `sopack extract` — source -> reviewable book.json (`SOPACK-1.0-PLAN.md` §3.5).

use std::path::{Path, PathBuf};

use serde::Serialize;
use sopack_book::Book;
use sopack_extract::{meta, ExtractOptions, Kind};

use crate::args::{Cli, ExtractArgs, MetaFlags};
use crate::exit::{CliError, EXIT_OK};
use crate::extract_progress::ExtractProgressAdapter;
use crate::output::{print_json, print_text};
use crate::progress_build::build_progress;

#[derive(Serialize)]
struct ExtractResult {
    out: String,
    book_code: Option<serde_json::Value>,
    lang: Option<serde_json::Value>,
    id_rule: String,
    stats: serde_json::Map<String, serde_json::Value>,
    year_from_source_note: Option<String>,
}

/// Every required-metadata problem `sopack_book::validate` can report:
/// `"book.<field> is required"` or the non-EGW author/year variant. `None`
/// unless *every* problem is one of these (a structural problem — a bad
/// id_rule, empty text, seq/chunk inconsistency — always means exit 3, even
/// mixed in with a genuinely missing field).
fn missing_metadata_fields(problems: &[String]) -> Option<Vec<String>> {
    if problems.is_empty() || !problems.iter().all(|p| p.contains("is required")) {
        return None;
    }
    Some(
        problems
            .iter()
            .filter_map(|p| {
                let rest = p.strip_prefix("book.")?;
                let field = rest.split(' ').next()?;
                Some(field.to_string())
            })
            .collect(),
    )
}

fn infer_kind(source: &Path) -> Kind {
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("epub") => Kind::Epub,
        Some("md") | Some("markdown") => Kind::Markdown,
        Some("json") => Kind::SopJson,
        _ => Kind::Text,
    }
}

/// Every CLI metadata flag as [`ExtractOptions`] (`chunk_limits` default) —
/// what [`meta::merge`] treats as the "explicit" side.
fn options_from_flags(flags: &MetaFlags) -> ExtractOptions {
    ExtractOptions {
        book_code: flags.book_code.clone(),
        lang: flags.lang.clone(),
        title: flags.title.clone(),
        author: flags.author.clone(),
        year: flags.year,
        corpus: flags.corpus.clone(),
        slug: flags.slug.clone(),
        book_pair: flags.book_pair.clone(),
        acquired_from: flags.acquired_from.clone(),
        rights: flags.rights.clone(),
        chunk_limits: Default::default(),
    }
}

/// Loads `<source>.meta.toml` (explicit `--meta` path, or auto-pickup) and
/// merges it with the CLI's explicit metadata flags
/// ([`meta::merge`]: agreement required on any field set by both).
fn resolve_options(
    source: &Path,
    explicit_meta_path: Option<&Path>,
    flags: &MetaFlags,
) -> Result<ExtractOptions, CliError> {
    let explicit = options_from_flags(flags);
    let sidecar_path = explicit_meta_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| {
            let mut s = source.as_os_str().to_owned();
            s.push(".meta.toml");
            PathBuf::from(s)
        });
    if !sidecar_path.is_file() {
        if explicit_meta_path.is_some() {
            return Err(CliError::input_invalid(format!(
                "--meta {} does not exist",
                sidecar_path.display()
            )));
        }
        return Ok(explicit);
    }
    let meta_file = meta::load(&sidecar_path)?;
    let sidecar = ExtractOptions::from_meta(&meta_file);
    Ok(meta::merge(&explicit, &sidecar)?)
}

fn extract_one(
    source: &Path,
    kind: Kind,
    opts: &ExtractOptions,
    page_kind: Option<&str>,
    id_rule: Option<&str>,
    progress_sink: &dyn sopack_progress::ProgressSink,
) -> Result<Book, CliError> {
    // No preflight metadata gate here on purpose. `meta::required_fields`/
    // `meta::check_complete` assume a field is either always options-only or
    // never options-only per kind, but `epub`'s `book_code` sits in
    // between: its inline-citation-code heuristic "only fires on a minority
    // of books" (`meta.rs`'s own doc comment), so a hard gate on it would
    // reject epubs that extract cleanly from OPF/citation metadata alone —
    // exactly the false positive this crate's own real-book conformance
    // fixtures (`wdys`, `come_out_of_her`, `seal_of_the_living_god`) hit
    // during development. The single, uniform, always-correct check is
    // `sopack_book::validate` on the real extracted Book, below — it knows
    // exactly what did and did not end up resolved, for every kind alike.
    let mut adapter = ExtractProgressAdapter::new(progress_sink);
    let mut book = sopack_extract::extract(source, kind, opts, &mut adapter)?;

    if let Some(pk) = page_kind {
        book.book.insert(
            "page_kind".to_string(),
            serde_json::Value::String(pk.to_string()),
        );
    }
    if let Some(rule) = id_rule {
        book.id_rule = rule.to_string();
    }

    let problems = sopack_book::validate(&book);
    if !problems.is_empty() {
        if let Some(missing) = missing_metadata_fields(&problems) {
            return Err(CliError::needs_metadata(format!(
                "{}: missing required metadata: {}",
                source.display(),
                missing.join(", ")
            ))
            .with_field(missing.join(",")));
        }
        return Err(CliError::input_invalid(format!(
            "{}: book.json is not valid:\n  - {}",
            source.display(),
            problems.join("\n  - ")
        )));
    }
    Ok(book)
}

pub fn run(cli: &Cli, args: &ExtractArgs) -> Result<i32, CliError> {
    let progress = build_progress(
        cli.progress,
        vec![sopack_progress::StageWeight::new("extract", 1.0)],
    );

    if args.meta_from_sidecars {
        return run_batch(cli, args, progress.as_ref());
    }

    let kind = args
        .kind
        .map(|k| k.to_extract_kind())
        .unwrap_or_else(|| infer_kind(&args.source));
    let opts = resolve_options(&args.source, args.meta.as_deref(), &args.meta_flags)?;

    let book = extract_one(
        &args.source,
        kind,
        &opts,
        args.page_kind.as_deref(),
        args.id_rule.as_deref(),
        progress.as_ref(),
    )?;
    progress.done();

    if let Some(parent) = args.out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    sopack_book::dump(&book, &args.out)?;

    let stats = book.stats.clone();
    let year_note = if opts.year.is_none() {
        book.book.get("year").filter(|v| !v.is_null()).map(|y| {
            format!(
                "year={y} was taken from the source file, not given by you — check it is \
                     the publication year, not a digital edition date"
            )
        })
    } else {
        None
    };

    let result = ExtractResult {
        out: args.out.display().to_string(),
        book_code: book.book.get("book_code").cloned(),
        lang: book.book.get("lang").cloned(),
        id_rule: book.id_rule.clone(),
        stats,
        year_from_source_note: year_note.clone(),
    };

    if cli.json {
        print_json(&serde_json::to_value(&result).unwrap());
    } else if !cli.quiet {
        print_text(&format!("wrote {}", args.out.display()));
        print_text(&format!(
            "  book_code={:?} lang={:?} id_rule={}",
            result.book_code, result.lang, result.id_rule
        ));
        if let Some(note) = &year_note {
            print_text(&format!("  NOTE: {note}"));
        }
    }
    Ok(EXIT_OK)
}

fn run_batch(
    cli: &Cli,
    args: &ExtractArgs,
    progress: &dyn sopack_progress::ProgressSink,
) -> Result<i32, CliError> {
    if !args.source.is_dir() {
        return Err(CliError::input_invalid(format!(
            "{}: --meta-from-sidecars requires a directory",
            args.source.display()
        )));
    }
    std::fs::create_dir_all(&args.out)?;

    let explicit = options_from_flags(&args.meta_flags);
    let pairs = meta::discover_sidecars(&args.source)?;

    let mut written = Vec::new();
    let mut errors = Vec::new();
    for (source_path, meta_file) in pairs {
        let kind = args
            .kind
            .map(|k| k.to_extract_kind())
            .unwrap_or_else(|| infer_kind(&source_path));
        let sidecar = ExtractOptions::from_meta(&meta_file);
        let opts = match meta::merge(&explicit, &sidecar) {
            Ok(o) => o,
            Err(e) => {
                errors.push(format!("{}: {}", source_path.display(), CliError::from(e)));
                continue;
            }
        };

        let stem = source_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("out");
        let out_path: PathBuf = args.out.join(format!("{stem}.book.json"));

        match extract_one(
            &source_path,
            kind,
            &opts,
            args.page_kind.as_deref(),
            args.id_rule.as_deref(),
            progress,
        ) {
            Ok(book) => {
                if let Err(e) = sopack_book::dump(&book, &out_path) {
                    errors.push(format!("{}: {}", source_path.display(), CliError::from(e)));
                    continue;
                }
                written.push(out_path.display().to_string());
            }
            Err(e) => errors.push(format!("{}: {}", source_path.display(), e)),
        }
    }
    progress.done();

    if cli.json {
        print_json(&serde_json::json!({"written": written, "errors": errors}));
    } else if !cli.quiet {
        for w in &written {
            print_text(&format!("wrote {w}"));
        }
        for e in &errors {
            eprintln!("error: {e}");
        }
    }

    if !errors.is_empty() && written.is_empty() {
        return Err(CliError::input_invalid(format!(
            "every source in {} failed to extract ({} error(s))",
            args.source.display(),
            errors.len()
        )));
    }
    Ok(EXIT_OK)
}
