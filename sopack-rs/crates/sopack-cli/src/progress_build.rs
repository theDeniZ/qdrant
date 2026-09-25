//! Builds the one [`sopack_progress::ProgressSink`] a command run reports
//! through, from the resolved `--progress` option and that command's stage
//! plan (`SOPACK-1.0-PLAN.md` §3.4's per-command table).

use sopack_progress::{NullSink, Progress, ProgressSink, StageWeight};

use crate::progress_opt::{ProgressOpt, ResolvedProgress};

pub fn build_progress(opt: ProgressOpt, plan: Vec<StageWeight>) -> Box<dyn ProgressSink> {
    match opt.resolve() {
        ResolvedProgress::Mode(mode) => Box::new(Progress::with_mode(plan, mode)),
        ResolvedProgress::Off => Box::new(NullSink),
    }
}
