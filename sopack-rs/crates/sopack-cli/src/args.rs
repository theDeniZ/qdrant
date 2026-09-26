//! The clap command tree. One `struct`/`enum` per command, so
//! `commands.rs`'s introspection (`sopack commands --json`) walks exactly
//! what `main.rs` dispatches on — there is no second, hand-maintained list
//! of subcommands/flags to drift from this one.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::progress_opt::ProgressOpt;

const EXIT_CODES_HELP: &str = "\
EXIT CODES:
    0  ok               success
    1  internal         an unexpected/internal error
    2  usage            bad command-line usage
    3  input_invalid    the given source/file/pack is malformed or unreadable
    4  needs_metadata   required metadata is missing and must be supplied
    5  calibration_failed  the calibration self-check scored below the contract's threshold
    6  resources        a required resource is missing (model, ONNX Runtime, memory, disk)
    7  interrupted      the run was interrupted; a partial/resumable state was left behind

Run `sopack commands --json` for the same table as data, plus every
subcommand's flags and result schema.";

#[derive(Parser, Debug)]
#[command(
    name = "sopack",
    version,
    about = "Prepare and verify corpus import packs for the bible-sop Qdrant collections",
    after_help = EXIT_CODES_HELP,
    propagate_version = true
)]
pub struct Cli {
    /// Print the command's result as one JSON document on stdout (progress,
    /// if any, still goes to stderr).
    #[arg(long, global = true)]
    pub json: bool,

    /// How progress is reported. `auto` picks a live bar on a terminal and
    /// throttled plain lines otherwise; `json` emits NDJSON on stderr
    /// regardless of what stderr is attached to; `none` disables progress
    /// reporting entirely.
    #[arg(long, global = true, value_enum, default_value_t = ProgressOpt::Auto)]
    pub progress: ProgressOpt,

    /// Embedding contract to use: an id this binary embeds (`e5-large-v1`,
    /// the default) or a directory containing its own `contract.toml`.
    #[arg(long, global = true, default_value = "e5-large-v1")]
    pub contract: String,

    /// Suppress the human-readable summary line most commands print in
    /// addition to their machine-readable result (has no effect with
    /// `--json`, which never prints that line anyway).
    #[arg(short = 'q', long = "quiet", global = true)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Source file (or folder of sources) -> reviewable book.json.
    Extract(ExtractArgs),
    /// Counts, damage, codes for a book.json.
    Inspect(InspectArgs),
    /// book.json(s) -> .sopack (embeds; the slow step).
    Pack(PackArgs),
    /// Embed the calibration fixture and report per-entry cosines.
    Calibrate(CalibrateArgs),
    /// Offline .sopack integrity check.
    Verify(VerifyArgs),
    /// Environment check (model, ONNX Runtime, cache, memory, threads).
    Doctor(DoctorArgs),
    /// Manage the local embedding model cache.
    Model(ModelArgs),
    /// Print a committed JSON Schema.
    Schema(SchemaArgs),
    /// Describe every subcommand, flag and exit code as JSON.
    Commands,
    /// Inspect the loaded embedding contract.
    Contract(ContractArgs),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum KindOpt {
    Epub,
    Markdown,
    Text,
    /// `--kind sop_json` — spelled with an underscore, matching
    /// `sopack_extract::Kind::SopJson::as_str()` and every error message
    /// that names it, rather than clap's default kebab-case `sop-json`.
    #[value(name = "sop_json")]
    SopJson,
}

impl KindOpt {
    pub fn to_extract_kind(self) -> sopack_extract::Kind {
        match self {
            KindOpt::Epub => sopack_extract::Kind::Epub,
            KindOpt::Markdown => sopack_extract::Kind::Markdown,
            KindOpt::Text => sopack_extract::Kind::Text,
            KindOpt::SopJson => sopack_extract::Kind::SopJson,
        }
    }
}

/// The 10 metadata override flags every chunkable extractor accepts (`sop_json`
/// accepts a subset — `sopack-extract::options::check_options` still refuses
/// the rest, so offering all 10 uniformly here and letting the library gate
/// them is the same "refuse rather than silently drop" posture as the
/// Python CLI, applied once instead of per-kind).
#[derive(Args, Debug, Clone, Default)]
pub struct MetaFlags {
    #[arg(long = "book-code")]
    pub book_code: Option<String>,
    #[arg(long)]
    pub lang: Option<String>,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub author: Option<String>,
    #[arg(long)]
    pub year: Option<i64>,
    #[arg(long)]
    pub corpus: Option<String>,
    #[arg(long)]
    pub slug: Option<String>,
    #[arg(long = "book-pair")]
    pub book_pair: Option<String>,
    #[arg(long = "acquired-from")]
    pub acquired_from: Option<String>,
    #[arg(long)]
    pub rights: Option<String>,
}

#[derive(Args, Debug)]
pub struct ExtractArgs {
    /// Source file, or a directory when `--meta-from-sidecars` is given.
    pub source: PathBuf,

    /// Source format; default: inferred from the file extension
    /// (.epub/.md,.markdown/.json/anything else -> text).
    #[arg(long, value_enum)]
    pub kind: Option<KindOpt>,

    /// Output book.json path (or output directory, with `--meta-from-sidecars`).
    #[arg(short = 'o', long)]
    pub out: PathBuf,

    /// A `<source>.meta.toml` sidecar to read metadata from (the same field
    /// names as the flags below). Without this flag, `<source>.meta.toml` is
    /// picked up automatically if it exists next to `source`.
    #[arg(long)]
    pub meta: Option<PathBuf>,

    /// Batch mode: `source` is a directory; every file in it is extracted
    /// with metadata from its own `<file>.meta.toml` sidecar, written to
    /// `--out` (a directory) as `<stem>.book.json`.
    #[arg(long)]
    pub meta_from_sidecars: bool,

    #[command(flatten)]
    pub meta_flags: MetaFlags,

    /// Overlay on `Book.book["page_kind"]` after extraction (every kind
    /// computes/fixes this internally; there is no per-kind flag for it).
    #[arg(long = "page-kind")]
    pub page_kind: Option<String>,

    /// Overlay on `Book.id_rule` after extraction (fixed by kind/profile by
    /// default; `sopack_book::validate` still checks it against the profile).
    #[arg(long = "id-rule")]
    pub id_rule: Option<String>,
}

#[derive(Args, Debug)]
pub struct InspectArgs {
    pub book_json: PathBuf,
}

#[derive(Args, Debug, Clone)]
pub struct EngineOpts {
    /// Execution device. `auto` self-verifies each candidate against the
    /// calibration fixture and falls back to `cpu` with a warning; an
    /// explicit non-cpu device refuses outright below the fixture threshold.
    #[arg(long, default_value = "cpu")]
    pub device: String,

    /// ORT intra/inter-op threads (default: available_parallelism()).
    #[arg(long)]
    pub threads: Option<usize>,

    /// Padded-token budget per batch (default: 512 on cpu, 8192 on a GPU/CoreML device).
    #[arg(long = "batch-tokens")]
    pub batch_tokens: Option<usize>,

    /// Override the resolved model directory (default: the contract's
    /// model under the platform cache dir — see `sopack model path`).
    #[arg(long = "model-dir")]
    pub model_dir: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct PackArgs {
    /// One or more book.json files (all the same profile; a mixed pack is refused).
    pub books: Vec<PathBuf>,

    #[arg(short = 'o', long)]
    pub out: PathBuf,

    #[arg(long)]
    pub profile: Option<String>,

    #[arg(long = "pack-id")]
    pub pack_id: Option<String>,

    /// Override the `created_by` provenance string (default:
    /// `"sopack <version> (rust) on <os> <release> <arch>"`). Mainly for
    /// reproducible test/CI runs that compare two packs byte-for-byte.
    #[arg(long = "created-by")]
    pub created_by: Option<String>,

    #[arg(long = "id-rule")]
    pub id_rule: Option<String>,

    #[command(flatten)]
    pub engine: EngineOpts,

    /// Discard an existing `<out>.sopack.partial/` checkpoint and start over
    /// (default: resume it when the inputs/contract/options still match).
    #[arg(long)]
    pub fresh: bool,
}

#[derive(Args, Debug)]
pub struct CalibrateArgs {
    #[command(flatten)]
    pub engine: EngineOpts,
}

#[derive(Args, Debug)]
pub struct VerifyArgs {
    pub pack: PathBuf,
}

#[derive(Args, Debug)]
pub struct DoctorArgs {
    /// Skip the real model load/embed/calibrate check and report cache/model
    /// file presence only. Must succeed on a clean CI runner with no model
    /// downloaded yet and no ONNX Runtime library present.
    #[arg(long)]
    pub quick: bool,

    /// Override the resolved model directory (default: under the platform cache dir).
    #[arg(long = "model-dir")]
    pub model_dir: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct ModelArgs {
    #[command(subcommand)]
    pub action: ModelAction,
}

#[derive(Subcommand, Debug)]
pub enum ModelAction {
    /// Download the contract's model files with byte progress + sha256 verification.
    Fetch(ModelDirArgs),
    /// Verify then hardlink/copy an existing model snapshot into the cache.
    Import(ModelImportArgs),
    /// Verify every model file's size and sha256 against the contract.
    Verify(ModelDirArgs),
    /// Print the resolved model directory (downloaded or not).
    Path(ModelDirArgs),
}

#[derive(Args, Debug)]
pub struct ModelDirArgs {
    /// Override the resolved model directory (default: under the platform cache dir).
    #[arg(long = "model-dir")]
    pub model_dir: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct ModelImportArgs {
    /// Directory holding an already-downloaded model snapshot (e.g. a
    /// fastembed/Hugging Face cache dir) to verify and import.
    pub dir: PathBuf,

    #[command(flatten)]
    pub model_dir: ModelDirArgs,
}

#[derive(Args, Debug)]
pub struct SchemaArgs {
    /// Schema name (see `--list`). Omit with `--list` to enumerate them.
    pub name: Option<String>,

    #[arg(long)]
    pub list: bool,
}

#[derive(Args, Debug)]
pub struct ContractArgs {
    #[command(subcommand)]
    pub action: ContractAction,
}

#[derive(Subcommand, Debug)]
pub enum ContractAction {
    /// Show the resolved contract's full detail.
    Show,
    /// List the contract ids this binary embeds.
    List,
}
