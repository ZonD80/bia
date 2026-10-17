//! llama.cpp wrapper over the prebuilt `llama-server` child.
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

use crate::config::{ModelCfg, SamplingCfg};
use crate::tune::LlamaPlan;

/// Token id from the server tokenizer. The id is what the completion API sends back.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct LlamaToken(pub i32);

impl LlamaToken {
    pub fn new(id: i32) -> Self {
        Self(id)
    }
}

/// How to load the GGUF later if boot skipped it because FMM was on.
#[derive(Clone)]
pub struct LocalSpec {
    pub path: PathBuf,
    pub cfg: ModelCfg,
    pub sampling: SamplingCfg,
    pub plan: LlamaPlan,
}

pub struct ModelInfo {
    pub name: String,
    pub arch: String,
    pub n_params: u64,
    pub size_bytes: u64,
    pub n_ctx_train: u32,
    pub n_layer: u32,
    pub experts: Option<String>,
    pub chat_template: String,
    pub has_bos: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Eos,
    Interrupted,
    MaxTokens,
    ContextFull,
}

pub struct GenStats {
    pub prompt_tokens: usize,
    pub reused_tokens: usize,
    pub generated: usize,
    pub prefill_secs: f64,
    pub gen_secs: f64,
    pub stop: Stop,
}

pub struct Llm {
    srv: crate::llama_srv::Server,
    sampling: SamplingCfg,
    cached: Vec<LlamaToken>,
    pub threads: i32,
    pub threads_batch: i32,
    pub device: String,
    pub notes: Vec<String>,
    pub mlock: bool,
    pub n_ctx: usize,
    pub info: ModelInfo,
}

impl Llm {
    pub fn load(
        path: &Path,
        cfg: &ModelCfg,
        sampling: &SamplingCfg,
        plan: &LlamaPlan,
        mut progress: impl FnMut(f32) -> bool + 'static,
    ) -> Result<Llm> {
        let mut plan = plan.clone();
        plan.ctx = cfg.ctx.max(512);
        if plan.mlock {
            // Inherited by llama-server so it can pin the GGUF.
            crate::sys::allow_mlock();
        }
        let (srv, notes) = crate::llama_srv::start(path, &plan, &mut progress)?;
        let meta = crate::gguf::read(path);
        let mut info = ModelInfo {
            name: if meta.name.is_empty() {
                path.file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default()
            } else {
                meta.name
            },
            arch: if meta.arch.is_empty() {
                "?".into()
            } else {
                meta.arch
            },
            n_params: meta.n_params,
            size_bytes: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
            n_ctx_train: meta.n_ctx_train,
            n_layer: meta.n_layer,
            experts: meta.experts,
            chat_template: {
                let live = srv.chat_template();
                if live.is_empty() {
                    meta.chat_template
                } else {
                    live
                }
            },
            has_bos: meta.has_bos,
        };
        if info.n_ctx_train == 0 {
            info.n_ctx_train = plan.ctx.max(512);
        }
        let n_ctx = srv.n_ctx().unwrap_or(plan.ctx).max(512);
        let device = srv.name.clone();
        Ok(Llm {
            srv,
            sampling: sampling.clone(),
            cached: vec![],
            threads: plan.threads,
            threads_batch: plan.threads_batch,
            device,
            notes,
            mlock: plan.mlock,
            n_ctx: n_ctx as usize,
            info,
        })
    }

    pub fn clamp_ctx(want: u32, n_ctx_train: u32) -> u32 {
        want.max(512).min(n_ctx_train.max(512))
    }

    /// Drop the KV cache, allocate `n_ctx` (clamped to the model's train window). On failure the
    /// previous window is loaded again. Returns the window that is actually live.
    pub fn reload_ctx(&mut self, n_ctx: u32) -> Result<u32> {
        let prev = self.n_ctx as u32;
        let want = Self::clamp_ctx(n_ctx, self.info.n_ctx_train);
        if want == prev {
            return Ok(want);
        }
        self.cached.clear();
        let live = self.srv.restart(want, &mut |_| true)?;
        self.n_ctx = live as usize;
        Ok(live)
    }

    pub fn tokenize(&self, text: &str, add_bos: bool) -> Result<Vec<LlamaToken>> {
        self.srv
            .tokenize(text, add_bos)
            .map(|ids| ids.into_iter().map(LlamaToken::new).collect())
    }

    pub fn cached_tokens(&self) -> usize {
        self.cached.len()
    }

    pub fn reset(&mut self) {
        self.cached.clear();
    }

    /// Generate a completion for `prompt` (full token sequence). The server reuses the common
    /// prefix of the previous call. `on_piece` gets each decoded piece; return false to stop.
    pub fn generate(
        &mut self,
        prompt: &[LlamaToken],
        max_new: usize,
        on_prefill: impl FnMut(usize, usize) -> bool,
        on_piece: impl FnMut(&str) -> bool,
    ) -> Result<(String, GenStats)> {
        if prompt.is_empty() {
            bail!("empty prompt");
        }
        if prompt.len() + 16 > self.n_ctx {
            bail!(
                "prompt ({} tokens) does not fit the context ({})",
                prompt.len(),
                self.n_ctx
            );
        }
        let ids: Vec<i32> = prompt.iter().map(|t| t.0).collect();
        let c = self
            .srv
            .complete(&ids, max_new, &self.sampling, on_prefill, on_piece)?;
        self.cached = prompt.to_vec();
        self.cached
            .extend(std::iter::repeat_n(LlamaToken::new(0), c.generated));
        let stop = if c.interrupted {
            Stop::Interrupted
        } else if c.truncated {
            Stop::ContextFull
        } else if c.eos {
            Stop::Eos
        } else {
            Stop::MaxTokens
        };
        Ok((
            c.text,
            GenStats {
                prompt_tokens: prompt.len(),
                reused_tokens: c.reused,
                generated: c.generated,
                prefill_secs: c.prefill_secs,
                gen_secs: c.gen_secs,
                stop,
            },
        ))
    }
}
