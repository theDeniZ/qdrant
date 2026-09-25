//! sopack 1.0 — M0 go/no-go spike (SOPACK-1.0-PLAN.md §2).
//!
//! Embeds the fixture texts with `ort` + `tokenizers` exactly the way Python
//! fastembed 0.8.0 does ("passage: " prefix, truncation 512, pad to the batch's
//! longest, mean pooling over the attention mask, L2 norm), then compares:
//!
//!   1. token ids            vs Python `tokenizers` (fixture py_ids / extra ids)
//!   2. one text at a time   vs the vectors stored in Qdrant, and vs Python
//!   3. fastembed batching   (input order, batch 32) vs stored
//!   4. length-sorted, token-budget batching vs stored and vs one-at-a-time
//!   5. thread scaling       1, 2, 4, … threads: throughput + bitwise identity
//!
//! Throwaway code: it proves or disproves the Rust path, nothing more.

use anyhow::{anyhow, Context, Result};
use ndarray::{Array2, Axis};
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;
use serde::Deserialize;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokenizers::{
    AddedToken, PaddingDirection, PaddingParams, PaddingStrategy, Tokenizer, TruncationParams,
};

const PREFIX: &str = "passage: ";
const BAR: f64 = 0.9999;

#[derive(Deserialize)]
struct Point {
    id: String,
    group: String,
    tag: String,
    text: String,
    stored: Vec<f32>,
    py_ids: Vec<u32>,
    py_vec: Vec<f32>,
}

#[derive(Deserialize)]
struct Extra {
    texts: std::collections::BTreeMap<String, String>,
    ids: std::collections::BTreeMap<String, Vec<u32>>,
}

/// The tokenizer configured exactly as fastembed's `load_tokenizer` does.
fn load_tokenizer(dir: &Path) -> Result<Tokenizer> {
    let mut tok = Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|e| anyhow!(e))?;
    let cfg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("tokenizer_config.json"))?)?;
    let model_cfg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("config.json"))?)?;
    let max_len = cfg["model_max_length"].as_u64().context("model_max_length")? as usize;
    tok.with_truncation(Some(TruncationParams { max_length: max_len, ..Default::default() }))
        .map_err(|e| anyhow!(e))?;
    if tok.get_padding().is_none() {
        tok.with_padding(Some(PaddingParams {
            strategy: PaddingStrategy::BatchLongest,
            direction: PaddingDirection::Right,
            pad_id: model_cfg["pad_token_id"].as_u64().unwrap_or(0) as u32,
            pad_token: cfg["pad_token"].as_str().context("pad_token")?.to_string(),
            ..Default::default()
        }));
    }
    // fastembed re-adds every entry of special_tokens_map.json as a special token
    let stm: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("special_tokens_map.json"))?)?;
    let mut added = vec![];
    for v in stm.as_object().context("special_tokens_map")?.values() {
        match v {
            serde_json::Value::String(s) => added.push(AddedToken::from(s.clone(), true)),
            serde_json::Value::Object(o) => added.push(
                AddedToken::from(o["content"].as_str().unwrap().to_string(), true)
                    .lstrip(o["lstrip"].as_bool().unwrap_or(false))
                    .rstrip(o["rstrip"].as_bool().unwrap_or(false))
                    .single_word(o["single_word"].as_bool().unwrap_or(false))
                    .normalized(o["normalized"].as_bool().unwrap_or(false)),
            ),
            _ => {}
        }
    }
    tok.add_special_tokens(added).map_err(|e| anyhow!(e))?;
    Ok(tok)
}

fn load_session(dir: &Path, threads: usize) -> Result<Session> {
    let e = |e: ort::Error<_>| anyhow!(e.to_string());
    Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3).map_err(e)?
        .with_intra_threads(threads).map_err(e)?
        .with_inter_threads(threads).map_err(e)?
        .commit_from_file(dir.join("model.onnx"))
        .context("load model.onnx")
}

/// Embed one batch: tokenise (pads to the batch's longest), run, mean-pool in
/// f64 (numpy promotes f32 * int64 mask to f64), L2-normalise, cast to f32.
fn embed_batch(sess: &mut Session, tok: &Tokenizer, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
    let enc = tok.encode_batch(texts.to_vec(), true).map_err(|e| anyhow!(e))?;
    let (b, l) = (enc.len(), enc[0].get_ids().len());
    let mut ids = Array2::<i64>::zeros((b, l));
    let mut mask = Array2::<i64>::zeros((b, l));
    for (i, e) in enc.iter().enumerate() {
        for (j, (&t, &m)) in e.get_ids().iter().zip(e.get_attention_mask()).enumerate() {
            ids[[i, j]] = t as i64;
            mask[[i, j]] = m as i64;
        }
    }
    let types = Array2::<i64>::zeros((b, l));
    let want_types = sess.inputs().iter().any(|i| i.name() == "token_type_ids");
    let outputs = if want_types {
        sess.run(ort::inputs![
            "input_ids" => Tensor::from_array(ids)?,
            "attention_mask" => Tensor::from_array(mask.clone())?,
            "token_type_ids" => Tensor::from_array(types)?,
        ])?
    } else {
        sess.run(ort::inputs![
            "input_ids" => Tensor::from_array(ids)?,
            "attention_mask" => Tensor::from_array(mask.clone())?,
        ])?
    };
    let hidden = outputs[0].try_extract_array::<f32>()?; // [b, l, d]
    let d = hidden.shape()[2];
    let mut out = Vec::with_capacity(b);
    for i in 0..b {
        let h = hidden.index_axis(Axis(0), i);
        let mut acc = vec![0f64; d];
        let mut n = 0f64;
        for j in 0..l {
            if mask[[i, j]] != 0 {
                n += 1.0;
                for k in 0..d {
                    acc[k] += h[[j, k]] as f64;
                }
            }
        }
        let n = n.max(1e-9);
        for a in acc.iter_mut() {
            *a /= n;
        }
        let norm = acc.iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-12);
        out.push(acc.iter().map(|x| (x / norm) as f32).collect());
    }
    Ok(out)
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let (mut ab, mut aa, mut bb) = (0f64, 0f64, 0f64);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (*x as f64, *y as f64);
        ab += x * y;
        aa += x * x;
        bb += y * y;
    }
    ab / (aa.sqrt() * bb.sqrt())
}

fn max_abs(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
}

/// Batches in input order, fixed size (what fastembed / sopack 0.1.x do).
fn embed_ordered(sess: &mut Session, tok: &Tokenizer, texts: &[String], bs: usize) -> Result<Vec<Vec<f32>>> {
    let mut out = vec![];
    for c in texts.chunks(bs) {
        let refs: Vec<&str> = c.iter().map(|s| s.as_str()).collect();
        out.extend(embed_batch(sess, tok, &refs)?);
    }
    Ok(out)
}

/// Length-sorted batches filled to a padded-token budget; original order restored.
fn embed_sorted(sess: &mut Session, tok: &Tokenizer, texts: &[String], budget: usize) -> Result<(Vec<Vec<f32>>, usize)> {
    let lens: Vec<usize> = texts
        .iter()
        .map(|t| tok.encode(t.as_str(), true).map(|e| e.get_ids().len()).map_err(|e| anyhow!(e)))
        .collect::<Result<_>>()?;
    let mut order: Vec<usize> = (0..texts.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(lens[i]));
    let mut out: Vec<Option<Vec<f32>>> = vec![None; texts.len()];
    let (mut i, mut nbatches) = (0, 0);
    while i < order.len() {
        let longest = lens[order[i]]; // sorted descending: first is the longest
        let n = (budget / longest).max(1).min(order.len() - i);
        let idx = &order[i..i + n];
        let refs: Vec<&str> = idx.iter().map(|&k| texts[k].as_str()).collect();
        for (k, v) in idx.iter().zip(embed_batch(sess, tok, &refs)?) {
            out[*k] = Some(v);
        }
        i += n;
        nbatches += 1;
    }
    Ok((out.into_iter().map(|v| v.unwrap()).collect(), nbatches))
}

fn summary(name: &str, cos: &[f64]) -> serde_json::Value {
    let min = cos.iter().cloned().fold(f64::INFINITY, f64::min);
    let mean = cos.iter().sum::<f64>() / cos.len() as f64;
    let below = cos.iter().filter(|&&c| c < BAR).count();
    println!("  {name:<42} n={:<4} min={min:.7} mean={mean:.7} below_bar={below}", cos.len());
    json!({"n": cos.len(), "min": min, "mean": mean, "below_bar": below, "pass": below == 0})
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let model_dir = PathBuf::from(args.get(1).context("usage: spike <model_dir> <fixture_dir> [threads...]")?);
    let fix = PathBuf::from(args.get(2).context("fixture dir")?);
    if args.get(3).map(|s| s.as_str()) == Some("sweep") {
        return sweep(&model_dir, &fix, args[4].parse()?, &args[5..]);
    }
    let thread_list: Vec<usize> = if args.len() > 3 {
        args[3..].iter().map(|s| s.parse().unwrap()).collect()
    } else {
        vec![1, 2, 4]
    };
    let cores = std::thread::available_parallelism()?.get();

    let points: Vec<Point> = serde_json::from_str(&std::fs::read_to_string(fix.join("points.json"))?)?;
    let extra: Extra = serde_json::from_str(&std::fs::read_to_string(fix.join("extra_tokens.json"))?)?;
    let texts: Vec<String> = points.iter().map(|p| format!("{PREFIX}{}", p.text)).collect();
    println!("fixture: {} points, available_parallelism={cores}", points.len());
    let mut report = json!({"available_parallelism": cores, "bar": BAR, "n_points": points.len()});

    // ── 1. token ids ────────────────────────────────────────────────────────
    let tok = load_tokenizer(&model_dir)?;
    let mut tok_bad = vec![];
    let mut max_tokens = 0;
    for (p, t) in points.iter().zip(&texts) {
        let ids = tok.encode(t.as_str(), true).map_err(|e| anyhow!(e))?.get_ids().to_vec();
        max_tokens = max_tokens.max(ids.len());
        if ids != p.py_ids {
            tok_bad.push(p.tag.clone());
        }
    }
    for (k, t) in &extra.texts {
        let ids = tok.encode(t.as_str(), true).map_err(|e| anyhow!(e))?.get_ids().to_vec();
        if &ids != &extra.ids[k] {
            tok_bad.push(format!("extra:{k}"));
        } else {
            println!("  tokens extra:{k:<6} {} ids identical", ids.len());
        }
    }
    println!("1. token ids: {} texts, {} mismatches, longest {} tokens",
             points.len() + extra.texts.len(), tok_bad.len(), max_tokens);
    report["tokens"] = json!({"n": points.len() + extra.texts.len(), "mismatches": tok_bad,
                              "longest_fixture_tokens": max_tokens});

    // ── 2. one at a time ────────────────────────────────────────────────────
    let t_all = *thread_list.iter().max().unwrap();
    let t0 = Instant::now();
    let mut sess = load_session(&model_dir, t_all)?;
    println!("model loaded in {:.1}s ({t_all} threads); inputs={:?}", t0.elapsed().as_secs_f64(),
             sess.inputs().iter().map(|i| i.name().to_string()).collect::<Vec<_>>());
    let t0 = Instant::now();
    let single = embed_ordered(&mut sess, &tok, &texts, 1)?;
    let single_s = t0.elapsed().as_secs_f64();
    println!("2. one-at-a-time ({:.1}s)", single_s);
    let cos_stored: Vec<f64> = single.iter().zip(&points).map(|(v, p)| cosine(v, &p.stored)).collect();
    let cos_py: Vec<f64> = single.iter().zip(&points).map(|(v, p)| cosine(v, &p.py_vec)).collect();
    let py_stored: Vec<f64> = points.iter().map(|p| cosine(&p.py_vec, &p.stored)).collect();
    let mut groups = serde_json::Map::new();
    let mut gnames: Vec<&str> = points.iter().map(|p| p.group.as_str()).collect();
    gnames.dedup();
    for g in &gnames {
        let c: Vec<f64> = points.iter().zip(&cos_stored).filter(|(p, _)| p.group == *g).map(|(_, c)| *c).collect();
        let min = c.iter().cloned().fold(f64::INFINITY, f64::min);
        groups.insert(g.to_string(), json!(min));
    }
    let worst: Vec<_> = {
        let mut v: Vec<(f64, &Point)> = cos_stored.iter().cloned().zip(&points).collect();
        v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        v.iter().take(5).map(|(c, p)| json!({"cos": c, "tag": p.tag, "id": p.id})).collect()
    };
    report["single_vs_stored"] = summary("rust single vs stored", &cos_stored);
    report["single_vs_python"] = summary("rust single vs python fastembed", &cos_py);
    report["python_vs_stored"] = summary("python fastembed vs stored (sanity)", &py_stored);
    report["single_min_by_group"] = serde_json::Value::Object(groups);
    report["single_worst5"] = json!(worst);
    report["single_seconds"] = json!(single_s);

    // ── 3. fastembed-style batching ─────────────────────────────────────────
    let t0 = Instant::now();
    let ordered = embed_ordered(&mut sess, &tok, &texts, 32)?;
    let ordered_s = t0.elapsed().as_secs_f64();
    println!("3. ordered batches of 32 ({:.1}s)", ordered_s);
    let c: Vec<f64> = ordered.iter().zip(&points).map(|(v, p)| cosine(v, &p.stored)).collect();
    report["ordered32_vs_stored"] = summary("rust ordered-32 vs stored", &c);
    report["ordered32_seconds"] = json!(ordered_s);

    // ── 4. length-sorted token-budget batching ──────────────────────────────
    for budget in [4096usize, 16384] {
        let t0 = Instant::now();
        let (sorted, nb) = embed_sorted(&mut sess, &tok, &texts, budget)?;
        let s = t0.elapsed().as_secs_f64();
        println!("4. sorted, budget {budget} tokens: {nb} batches ({s:.1}s)");
        let cs: Vec<f64> = sorted.iter().zip(&points).map(|(v, p)| cosine(v, &p.stored)).collect();
        let c1: Vec<f64> = sorted.iter().zip(&single).map(|(v, s)| cosine(v, s)).collect();
        let mx = sorted.iter().zip(&single).map(|(a, b)| max_abs(a, b)).fold(0f32, f32::max);
        report[format!("sorted{budget}")] = json!({
            "batches": nb, "seconds": s,
            "vs_stored": summary(&format!("rust sorted-{budget} vs stored"), &cs),
            "vs_single": summary(&format!("rust sorted-{budget} vs rust single"), &c1),
            "max_abs_diff_vs_single": mx,
        });
    }
    drop(sess);

    // ── 5. thread scaling on the bench group (sorted, budget 8192) ──────────
    let bench: Vec<String> = points.iter().zip(&texts).filter(|(p, _)| p.group == "bench").map(|(_, t)| t.clone()).collect();
    let bench_tokens: usize = bench.iter().map(|t| tok.encode(t.as_str(), true).unwrap().get_ids().len()).sum();
    println!("5. thread scaling: {} bench texts, {bench_tokens} tokens", bench.len());
    let mut reference: Option<Vec<Vec<f32>>> = None;
    let mut rows = vec![];
    for &th in &thread_list {
        let mut sess = load_session(&model_dir, th)?;
        embed_batch(&mut sess, &tok, &[texts[0].as_str()])?; // warm-up
        let t0 = Instant::now();
        let (v, _) = embed_sorted(&mut sess, &tok, &bench, 8192)?;
        let s = t0.elapsed().as_secs_f64();
        let (ident, mx) = match &reference {
            None => (true, 0.0),
            Some(r) => {
                let mx = v.iter().zip(r).map(|(a, b)| max_abs(a, b)).fold(0f32, f32::max);
                (mx == 0.0, mx)
            }
        };
        let min_cos = match &reference {
            None => 1.0,
            Some(r) => v.iter().zip(r).map(|(a, b)| cosine(a, b)).fold(f64::INFINITY, f64::min),
        };
        println!("   threads={th:<2} {s:>7.1}s  {:>6.2} blocks/s  {:>7.0} tokens/s  identical_to_first={ident} max_abs={mx:e} min_cos={min_cos:.9}",
                 bench.len() as f64 / s, bench_tokens as f64 / s);
        rows.push(json!({"threads": th, "seconds": s, "blocks_per_s": bench.len() as f64 / s,
                         "tokens_per_s": bench_tokens as f64 / s, "bitwise_identical_to_first": ident,
                         "max_abs_diff_vs_first": mx, "min_cos_vs_first": min_cos}));
        if reference.is_none() {
            reference = Some(v);
        }
    }
    report["thread_scaling"] = json!({"bench_texts": bench.len(), "bench_tokens": bench_tokens, "rows": rows});

    std::fs::create_dir_all("results")?;
    let out = format!("results/m0-{}.json", std::env::consts::ARCH);
    std::fs::write(&out, serde_json::to_string_pretty(&report)?)?;
    println!("report → {out}");
    Ok(())
}

/// Budget sweep on the bench group: which padded-token budget is fastest on this CPU?
/// Budget 0 = one text at a time.
fn sweep(model_dir: &Path, fix: &Path, threads: usize, budgets: &[String]) -> Result<()> {
    let points: Vec<Point> = serde_json::from_str(&std::fs::read_to_string(fix.join("points.json"))?)?;
    let tok = load_tokenizer(model_dir)?;
    let bench: Vec<String> = points.iter().filter(|p| p.group == "bench").map(|p| format!("{PREFIX}{}", p.text)).collect();
    let stored: Vec<&Vec<f32>> = points.iter().filter(|p| p.group == "bench").map(|p| &p.stored).collect();
    let mut sess = load_session(model_dir, threads)?;
    embed_batch(&mut sess, &tok, &[bench[0].as_str()])?;
    let mut rows = vec![];
    for b in budgets {
        let budget: usize = b.parse()?;
        let t0 = Instant::now();
        let (v, nb) = if budget == 0 { (embed_ordered(&mut sess, &tok, &bench, 1)?, bench.len()) } else { embed_sorted(&mut sess, &tok, &bench, budget)? };
        let s = t0.elapsed().as_secs_f64();
        let min = v.iter().zip(&stored).map(|(a, b)| cosine(a, b)).fold(f64::INFINITY, f64::min);
        println!("budget={budget:<6} batches={nb:<4} {s:>6.1}s {:>5.2} blocks/s min_cos_vs_stored={min:.7}", bench.len() as f64 / s);
        rows.push(json!({"budget": budget, "batches": nb, "seconds": s, "blocks_per_s": bench.len() as f64 / s, "min_cos_vs_stored": min}));
    }
    std::fs::create_dir_all("results")?;
    std::fs::write(format!("results/m0-sweep-{}-t{threads}.json", std::env::consts::ARCH), serde_json::to_string_pretty(&json!({"threads": threads, "rows": rows}))?)?;
    Ok(())
}
