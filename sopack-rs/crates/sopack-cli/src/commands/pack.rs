//! `sopack pack` — book.json(s) -> `.sopack` (`SOPACK-1.0-PLAN.md` §3.4/§3.6,
//! `SOPACK-2-FORMAT.md`). See the module docs below each phase for the
//! order this follows and how resume is implemented.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sopack_book::{to_payload, uid, Block, Book};
use sopack_format::{BookEntry, EmbeddingProvenance, PackWriter, ProbeEntryInput};
use sopack_progress::{ProgressSink, StageWeight, Unit};

use crate::args::{Cli, PackArgs};
use crate::contract_load::load_contract;
use crate::engine_build::build_engine;
use crate::exit::{CliError, EXIT_OK};
use crate::output::{print_json, print_text};
use crate::progress_build::build_progress;
use crate::provenance::{default_created_by, default_pack_id};

/// How many blocks are embedded and written between `PackWriter::checkpoint()`
/// calls — the resumability granularity. Small enough that
/// `SOPACK_TEST_ABORT_AFTER_BATCHES` gives a real crash-then-resume test on
/// even a small book, independent of the embedding engine's own internal
/// token-budget batching (`sopack_embed::batch`), which this is orthogonal
/// to: one CLI "batch" here can still turn into several ORT calls inside
/// `Engine::embed`.
const CHECKPOINT_BATCH_BLOCKS: usize = 16;

const FINGERPRINT_SCHEMA: &str = "sopack-cli.pack-resume/1";
const FINGERPRINT_FILE: &str = "cli-state.json";

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct PackFingerprint {
    schema: String,
    books: Vec<(String, String)>,
    contract_id: String,
    contract_sha256: String,
    profile: String,
    id_rule: String,
    device: String,
    threads: Option<usize>,
    batch_tokens: Option<usize>,
}

fn checkpoint_dir_for(out_path: &Path) -> PathBuf {
    let mut s = out_path.as_os_str().to_owned();
    s.push(".sopack.partial");
    PathBuf::from(s)
}

fn read_fingerprint(dir: &Path) -> Option<PackFingerprint> {
    let bytes = std::fs::read(dir.join(FINGERPRINT_FILE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_fingerprint(dir: &Path, fp: &PackFingerprint) -> Result<(), CliError> {
    let bytes = serde_json::to_vec_pretty(fp)?;
    std::fs::write(dir.join(FINGERPRINT_FILE), bytes)?;
    Ok(())
}

struct LoadedBook {
    path: PathBuf,
    sha256: String,
    book: Book,
}

fn load_books(paths: &[PathBuf]) -> Result<Vec<LoadedBook>, CliError> {
    if paths.is_empty() {
        return Err(CliError::usage("pack: no books given"));
    }
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        let book = sopack_book::load(p)?;
        let problems = sopack_book::validate(&book);
        if !problems.is_empty() {
            return Err(CliError::input_invalid(format!(
                "{}: book.json is not valid, refusing to pack:\n  - {}",
                p.display(),
                problems.join("\n  - ")
            )));
        }
        let sha256 = sopack_book::book_sha256(p)?;
        out.push(LoadedBook {
            path: p.clone(),
            sha256,
            book,
        });
    }
    Ok(out)
}

fn resolve_profile(
    books: &[LoadedBook],
    given: Option<&str>,
) -> Result<&'static sopack_contract::Profile, CliError> {
    let declared: std::collections::BTreeSet<&str> =
        books.iter().map(|b| b.book.profile.as_str()).collect();
    let name = if let Some(p) = given {
        if declared.iter().any(|d| *d != p) {
            return Err(CliError::input_invalid(format!(
                "--profile {p:?} was given but book(s) declare {declared:?}"
            )));
        }
        p.to_string()
    } else if declared.len() == 1 {
        (*declared.iter().next().unwrap()).to_string()
    } else {
        return Err(CliError::input_invalid(format!(
            "books declare different profiles {declared:?} — pack them separately, one \
             profile per .sopack"
        )));
    };
    sopack_book::get_profile(&name).map_err(CliError::input_invalid)
}

fn resolve_id_rule(
    books: &[LoadedBook],
    profile: &sopack_contract::Profile,
    given: Option<&str>,
) -> Result<String, CliError> {
    let rule = if let Some(r) = given {
        r.to_string()
    } else {
        let declared: std::collections::BTreeSet<&str> =
            books.iter().map(|b| b.book.id_rule.as_str()).collect();
        if declared.len() > 1 {
            return Err(CliError::input_invalid(format!(
                "books declare different id_rules {declared:?} in one pack — a .sopack \
                 carries exactly one id_rule; pack them separately or pass --id-rule"
            )));
        }
        declared
            .into_iter()
            .next()
            .unwrap_or(&profile.default_id_rule)
            .to_string()
    };
    if !profile.id_rules.iter().any(|r| r == &rule) {
        return Err(CliError::input_invalid(format!(
            "id_rule {rule:?} is not valid for profile {:?} (allowed: {:?})",
            profile.name, profile.id_rules
        )));
    }
    Ok(rule)
}

/// Additive `titles.json` fragment — `sopack.pack._titles_fragment`. `bible`
/// has no title table.
fn titles_fragment(profile_name: &str, book_entries: &[BookEntry]) -> Option<serde_json::Value> {
    if profile_name != "sop" {
        return None;
    }
    let mut frag = serde_json::Map::new();
    for e in book_entries {
        let Some(lang) = &e.lang else { continue };
        let lang_map = frag
            .entry(lang.clone())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        let lang_obj = lang_map.as_object_mut().unwrap();
        let entry = lang_obj
            .entry(e.book_code.clone())
            .or_insert_with(|| serde_json::json!({"titles": []}));
        let entry_obj = entry.as_object_mut().unwrap();
        if let Some(title) = &e.title {
            let titles = entry_obj
                .get_mut("titles")
                .and_then(|v| v.as_array_mut())
                .unwrap();
            if !titles.iter().any(|t| t.as_str() == Some(title.as_str())) {
                titles.push(serde_json::Value::String(title.clone()));
            }
        }
        if let Some(author) = &e.author {
            entry_obj
                .entry("author")
                .or_insert_with(|| serde_json::Value::String(author.clone()));
        }
        if let Some(year) = e.year {
            entry_obj
                .entry("year")
                .or_insert_with(|| serde_json::Value::from(year));
        }
        if let Some(corpus) = &e.corpus {
            entry_obj
                .entry("corpus")
                .or_insert_with(|| serde_json::Value::String(corpus.clone()));
        }
        if lang != "en" {
            if let Some(pair) = &e.book_pair {
                if let Some((_, en_code)) = pair.split_once('/') {
                    let en_code = en_code.trim();
                    if !en_code.is_empty() {
                        entry_obj
                            .entry("en_code")
                            .or_insert_with(|| serde_json::Value::String(en_code.to_string()));
                    }
                }
            }
        }
    }
    Some(serde_json::Value::Object(frag))
}

#[derive(Serialize)]
struct PackResult {
    out: String,
    pack_id: String,
    profile: String,
    id_rule: String,
    points: u64,
    books: u64,
    device: String,
    calibration_min_cosine: f64,
    calibration_mean_cosine: f64,
    resumed: bool,
}

pub fn run(cli: &Cli, args: &PackArgs) -> Result<i32, CliError> {
    let contract = load_contract(&cli.contract)?;
    let loaded = load_books(&args.books)?;
    let profile = resolve_profile(&loaded, args.profile.as_deref())?;
    let id_rule = resolve_id_rule(&loaded, profile, args.id_rule.as_deref())?;
    let device_str = args.engine.device.clone();
    // Validated again inside build_engine; validated here too so a bad
    // --device value is reported before any checkpoint directory is touched.
    let _: sopack_embed::device::Device = device_str.parse().map_err(CliError::usage)?;

    let fingerprint = PackFingerprint {
        schema: FINGERPRINT_SCHEMA.to_string(),
        books: loaded
            .iter()
            .map(|b| (b.path.display().to_string(), b.sha256.clone()))
            .collect(),
        contract_id: contract.id.clone(),
        contract_sha256: contract.contract_sha256(),
        profile: profile.name.clone(),
        id_rule: id_rule.clone(),
        device: device_str.clone(),
        threads: args.engine.threads,
        batch_tokens: args.engine.batch_tokens,
    };

    let checkpoint_dir = checkpoint_dir_for(&args.out);
    let mut resumed = false;
    if checkpoint_dir.exists() {
        if args.fresh {
            std::fs::remove_dir_all(&checkpoint_dir)?;
        } else {
            match read_fingerprint(&checkpoint_dir) {
                Some(existing) if existing == fingerprint => resumed = true,
                Some(_) => {
                    return Err(CliError::input_invalid(format!(
                        "a checkpoint at {} was started with different inputs/options — \
                         re-run with --fresh to discard it and start over",
                        checkpoint_dir.display()
                    )))
                }
                None => {
                    return Err(CliError::input_invalid(format!(
                        "{} exists but has no readable {} — re-run with --fresh",
                        checkpoint_dir.display(),
                        FINGERPRINT_FILE
                    )))
                }
            }
        }
    }

    let progress = build_progress(
        cli.progress,
        vec![
            StageWeight::new("verify", 1.0),
            StageWeight::new("load_model", 1.0),
            StageWeight::new("calibrate", 1.0),
            StageWeight::new("embed", 10.0),
            StageWeight::new("write", 1.0),
        ],
    );

    let (mut engine, device_report, _model_dir) =
        build_engine(&contract, &args.engine, progress.as_ref())?;

    let fixture = contract.fixture.as_ref().ok_or_else(|| {
        CliError::resources(format!(
            "contract {:?} has no calibration fixture loaded",
            contract.id
        ))
    })?;
    let embed_fixture: sopack_embed::calibration::CalibrationFixture = fixture.into();
    let calib = match device_report.calibration {
        Some(c) => c,
        None => engine.calibrate(&embed_fixture, progress.as_ref())?,
    };
    let threshold = contract.calibration.pack_min_cosine;
    if !calib.passes(threshold) {
        return Err(CliError::calibration_failed(format!(
            "calibration self-check FAILED: min cosine {:.8} < {threshold} (mean {:.8}, n={}) \
             — refusing to embed any book",
            calib.min_cosine, calib.mean_cosine, calib.n
        )));
    }

    let provenance = EmbeddingProvenance {
        runtime: engine.runtime_string(env!("CARGO_PKG_VERSION")),
        device: engine.device().to_string(),
        threads: engine.threads() as u32,
        batch_tokens: engine.batch_tokens() as u32,
    };

    let mut writer = if resumed {
        PackWriter::resume(&args.out, &contract)?
    } else {
        let w = PackWriter::create(
            &args.out,
            &contract,
            &profile.name,
            args.pack_id
                .clone()
                .unwrap_or_else(|| default_pack_id(&profile.name)),
            args.created_by.clone().unwrap_or_else(default_created_by),
            Some(id_rule.clone()),
            provenance,
        )?;
        write_fingerprint(&checkpoint_dir, &fingerprint)?;
        w
    };

    // Flatten every (book, block) pair, in the order books/blocks were
    // given — resuming skips exactly `writer.count()` of them, matching how
    // many were durably appended before the crash.
    let mut flat: Vec<(usize, usize)> = Vec::new();
    for (bi, lb) in loaded.iter().enumerate() {
        if lb.book.blocks.is_empty() {
            progress.warn(&format!("{}: no blocks, skipping", lb.path.display()));
            continue;
        }
        for i in 0..lb.book.blocks.len() {
            flat.push((bi, i));
        }
    }
    let already = writer.count() as usize;
    if already > flat.len() {
        return Err(CliError::input_invalid(format!(
            "checkpoint at {} already holds {already} points, but only {} are expected from \
             these books — refusing to resume; use --fresh",
            checkpoint_dir.display(),
            flat.len()
        )));
    }

    // ONE "embed" stage for the whole run, sized in real subword tokens of
    // exactly the blocks still to embed (a resumed run only counts what is
    // left). Each checkpoint chunk below calls `engine.embed()`, which would
    // otherwise open and close its OWN "embed" stage per chunk — the overall
    // percentage then plateaued for the whole embed phase of any book over
    // one chunk (bench/README.md "Progress engine finding"). `AdvanceOnly`
    // forwards the chunk's token advances into this stage and swallows its
    // stage boundaries.
    let remaining_texts: Vec<String> = flat[already..]
        .iter()
        .map(|&(bi, ei)| block_text(&loaded[bi].book, &loaded[bi].book.blocks[ei], profile))
        .collect::<Result<_, CliError>>()?;
    let embed_total = engine.count_tokens(&remaining_texts)?;
    drop(remaining_texts);
    progress.stage_start("embed", embed_total, Unit::Tokens);
    let chunk_progress = AdvanceOnly(progress.as_ref());

    let abort_after: Option<usize> = std::env::var("SOPACK_TEST_ABORT_AFTER_BATCHES")
        .ok()
        .and_then(|v| v.parse().ok());
    let mut batches_done = 0usize;

    for chunk in flat[already..].chunks(CHECKPOINT_BATCH_BLOCKS) {
        let mut texts = Vec::with_capacity(chunk.len());
        let mut payloads = Vec::with_capacity(chunk.len());
        let mut uids = Vec::with_capacity(chunk.len());
        for &(bi, ei) in chunk {
            let book = &loaded[bi].book;
            let block: &Block = &book.blocks[ei];
            let payload = to_payload(book, block)?;
            let u = uid(book, block)?;
            let text = payload
                .get(profile.text_field.as_str())
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            texts.push(text);
            payloads.push(payload);
            uids.push(u);
        }
        let vectors = engine.embed(&texts, sopack_embed::spec::Role::Passage, &chunk_progress)?;
        for (i, vector) in vectors.iter().enumerate() {
            writer.add(&uids[i], payloads[i].clone(), vector)?;
        }
        writer.checkpoint()?;
        batches_done += 1;

        if let Some(n) = abort_after {
            if batches_done >= n {
                eprintln!(
                    "SOPACK_TEST_ABORT_AFTER_BATCHES={n}: simulating a crash after {batches_done} \
                     batch(es) — {} point(s) written, no finish()",
                    writer.count()
                );
                std::process::exit(crate::exit::CliErrorCode::Interrupted.exit());
            }
        }
    }
    progress.stage_end();

    let mut book_entries = Vec::new();
    for lb in loaded.iter() {
        if lb.book.blocks.is_empty() {
            continue;
        }
        let meta = &lb.book.book;
        let get = |k: &str| -> Option<String> {
            meta.get(k).and_then(|v| v.as_str()).map(str::to_string)
        };
        let get_year = || -> Option<i64> { meta.get("year").and_then(|v| v.as_i64()) };
        // Computed independently of how much of this run was resumed (see
        // the module doc): `sopack_book::uid`'s pre-hash string, uuid5'd the
        // same way `PackWriter::add` derives a point's id internally, so
        // the first point's id is always this book's first_id — not
        // whichever point happened to be embedded first *in this process*.
        let first_id = uid(&lb.book, &lb.book.blocks[0])
            .ok()
            .map(|u| uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, u.as_bytes()).to_string());
        book_entries.push(BookEntry {
            book_code: get("book_code").unwrap_or_default(),
            lang: get("lang"),
            points: lb.book.blocks.len() as u64,
            first_id,
            title: get("title"),
            author: get("author"),
            year: get_year(),
            corpus: get("corpus"),
            slug: get("slug"),
            book_pair: get("book_pair"),
            id_rule: id_rule.clone(),
            book_sha256: Some(lb.sha256.clone()),
        });
    }

    let probe_entries: Vec<ProbeEntryInput> = fixture
        .entries
        .iter()
        .map(|e| ProbeEntryInput {
            id: e.id.clone(),
            profile: e.profile.clone(),
        })
        .collect();

    let pack_id = writer.pack_id().to_string();
    writer.set_books(book_entries.clone());
    writer.set_titles(titles_fragment(&profile.name, &book_entries));
    writer.set_probe(
        probe_entries,
        calib.probe_vectors.clone(),
        calib.n as u64,
        calib.min_cosine,
        calib.mean_cosine,
    )?;

    progress.stage_start("write", 1, Unit::Items);
    let out_path = writer.finish()?;
    progress.advance(1);
    progress.stage_end();
    progress.done();

    let result = PackResult {
        out: out_path.display().to_string(),
        pack_id,
        profile: profile.name.clone(),
        id_rule,
        points: book_entries.iter().map(|b| b.points).sum(),
        books: book_entries.len() as u64,
        device: engine.device().to_string(),
        calibration_min_cosine: calib.min_cosine,
        calibration_mean_cosine: calib.mean_cosine,
        resumed,
    };

    if cli.json {
        print_json(&serde_json::to_value(&result).unwrap());
    } else if !cli.quiet {
        print_text(&format!(
            "wrote {} — {} points, {} books (device={}, min_cosine={:.6})",
            result.out, result.points, result.books, result.device, result.calibration_min_cosine
        ));
    }
    Ok(EXIT_OK)
}

/// The text a block is embedded from: the profile's text field of its payload
/// (the same value the chunk loop embeds).
fn block_text(
    book: &sopack_book::Book,
    block: &Block,
    profile: &sopack_contract::Profile,
) -> Result<String, CliError> {
    let payload = to_payload(book, block)?;
    Ok(payload
        .get(profile.text_field.as_str())
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string())
}

/// Forwards token advances and warnings into the caller's already-open
/// "embed" stage, and ignores the per-call stage boundaries `Engine::embed`
/// emits — so one stage spans every checkpoint chunk.
struct AdvanceOnly<'a>(&'a dyn ProgressSink);

impl ProgressSink for AdvanceOnly<'_> {
    fn stage_start(&self, _stage: &str, _total: u64, _unit: Unit) {}
    fn advance(&self, n: u64) {
        self.0.advance(n);
    }
    fn warn(&self, msg: &str) {
        self.0.warn(msg);
    }
    fn stage_end(&self) {}
    fn done(&self) {}
}
