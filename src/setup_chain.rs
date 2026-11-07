//! 「はじめる」 prepares everything live translation needs as one chain of steps.
//!
//! `Chain` is only the state: the steps still to run, the one that runs, how it
//! ended. The frame loop starts steps through `App::run_chain`, so a test feeds
//! the chain the events the installers send and no installer ever starts.
use super::*;
use plamo_translate::{
    installer::{self, InstallRequest, Progress},
    nemotron_setup, punctuation_setup, speech_setup,
};
use std::collections::VecDeque;

/// What the status line says on the live screen the chain opens.
pub(super) const READY_STATUS: &str = "準備ができました。";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Step {
    Translation,
    Punctuation,
    Speech,
    Voice,
}

impl Step {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Translation => "翻訳モデル",
            Self::Punctuation => "英語の文章区切り補助",
            Self::Speech => "音声認識",
            Self::Voice => "読み上げの声",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Running,
    Failed,
    Cancelled,
}

#[derive(Debug)]
pub(super) struct Chain {
    done: Vec<Step>,
    /// The step that was started and has not finished. After a stop, the step that stopped.
    current: Option<Step>,
    pending: VecDeque<Step>,
    outcome: Outcome,
    error: String,
}

impl Chain {
    pub(super) fn new(steps: impl IntoIterator<Item = Step>) -> Self {
        Self {
            done: Vec::new(),
            current: None,
            pending: steps.into_iter().collect(),
            outcome: Outcome::Running,
            error: String::new(),
        }
    }

    pub(super) fn current(&self) -> Option<Step> {
        self.current
    }

    /// The step the screen names: the one that runs, or the one that starts next.
    pub(super) fn active(&self) -> Option<Step> {
        self.current.or(self.pending.front().copied())
    }

    #[cfg(test)]
    pub(super) fn done(&self) -> &[Step] {
        &self.done
    }

    pub(super) fn outcome(&self) -> Outcome {
        self.outcome
    }

    pub(super) fn is_running(&self) -> bool {
        self.outcome == Outcome::Running
    }

    pub(super) fn is_finished(&self) -> bool {
        self.is_running() && self.current.is_none() && self.pending.is_empty()
    }

    /// The detailed error of the step that failed.
    pub(super) fn failure(&self) -> Option<&str> {
        (self.outcome == Outcome::Failed).then_some(self.error.as_str())
    }

    /// The number of the step now running and how many there are in all.
    pub(super) fn position(&self) -> (usize, usize) {
        let total = self.done.len() + self.current.iter().len() + self.pending.len();
        ((self.done.len() + 1).min(total), total)
    }

    /// The next step to start, once the one running has finished.
    pub(super) fn take_next(&mut self) -> Option<Step> {
        if !self.is_running() || self.current.is_some() {
            return None;
        }
        self.current = self.pending.pop_front();
        self.current
    }

    fn waits_for(&self, step: Step) -> bool {
        self.is_running() && self.current == Some(step)
    }

    /// Only the step the chain waits for counts, so a late event of another one changes nothing.
    pub(super) fn complete(&mut self, step: Step) -> bool {
        let waits = self.waits_for(step);
        if waits {
            self.current = None;
            self.done.push(step);
        }
        waits
    }

    pub(super) fn fail(&mut self, step: Step, error: String) -> bool {
        let waits = self.waits_for(step);
        if waits {
            self.outcome = Outcome::Failed;
            self.error = error;
        }
        waits
    }

    pub(super) fn cancel(&mut self, step: Step) -> bool {
        let waits = self.waits_for(step);
        if waits {
            self.outcome = Outcome::Cancelled;
        }
        waits
    }

    /// Start again from the step that stopped. The steps already done stay done.
    pub(super) fn retry(&mut self) -> bool {
        if self.is_running() {
            return false;
        }
        self.outcome = Outcome::Running;
        self.error.clear();
        if let Some(step) = self.current.take() {
            self.pending.push_front(step);
        }
        true
    }
}

/// Where each step keeps its files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Roots {
    pub engine: PathBuf,
    pub punctuation: PathBuf,
    pub nemotron: PathBuf,
    pub voice: PathBuf,
}

impl Roots {
    fn on_this_pc(engine: PathBuf) -> Self {
        let data = data_root();
        Self {
            engine,
            punctuation: data.join("text-punctuation"),
            nemotron: data.join("nemotron"),
            voice: data.join("speech"),
        }
    }

    /// Everything under one folder, so a preview or a test chooses what is saved.
    pub(super) fn under(folder: &Path) -> Self {
        Self {
            engine: folder.join("engine"),
            punctuation: folder.join("text-punctuation"),
            nemotron: folder.join("nemotron"),
            voice: folder.join("speech"),
        }
    }
}

/// What the button prepares: the steps in order, and what each still has to download.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Plan {
    pub steps: Vec<(Step, u64)>,
}

impl Plan {
    pub(super) fn steps(&self) -> Vec<Step> {
        self.steps.iter().map(|(step, _)| *step).collect()
    }

    pub(super) fn total(&self) -> u64 {
        self.steps.iter().map(|(_, bytes)| bytes).sum()
    }

    pub(super) fn includes(&self, step: Step) -> bool {
        self.steps.iter().any(|(planned, _)| *planned == step)
    }
}

/// What a plan depends on. Measuring it reads files, so it is made again only
/// when this changes or the files do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PlanKey {
    model: String,
    runtime: &'static str,
    store: String,
    steps: Vec<Step>,
    nemotron: &'static str,
}

impl App {
    pub(super) fn install_request(&self) -> Option<InstallRequest> {
        let model = self.setup_model()?;
        let backend = self.setup.recommendation.as_ref()?.backend.as_str();
        Some(InstallRequest {
            repo: model.repo,
            revision: model.revision,
            filename: model.filename,
            bytes: model.bytes,
            sha256: model.sha256,
            backend: backend.into(),
            install_root: self.model_store_root(),
        })
    }

    /// The steps the settings ask for: the translation model always, the rest as chosen.
    fn planned_steps(&self) -> Vec<Step> {
        let mut steps = vec![Step::Translation];
        for (wanted, step) in [
            (self.settings.english_punctuation, Step::Punctuation),
            (self.settings.asr_backend == "nemotron", Step::Speech),
            (self.settings.readout_enabled, Step::Voice),
        ] {
            if wanted {
                steps.push(step);
            }
        }
        steps
    }

    pub(super) fn plan_key(&self) -> Option<PlanKey> {
        Some(PlanKey {
            model: self.setup_model()?.id,
            runtime: self.setup.recommendation.as_ref()?.backend.as_str(),
            store: self.settings.model_store.clone(),
            steps: self.planned_steps(),
            nemotron: self.nemotron_backend(),
        })
    }

    /// The plan with what is saved under `roots`.
    pub(super) fn plan_under(&self, roots: &Roots) -> Option<Plan> {
        let mut request = self.install_request()?;
        request.install_root = roots.engine.clone();
        let steps = self
            .planned_steps()
            .into_iter()
            .map(|step| {
                let bytes = match step {
                    Step::Translation => installer::pending_download_bytes(&request),
                    Step::Punctuation => {
                        punctuation_setup::pending_download_bytes(&roots.punctuation)
                    }
                    Step::Speech => nemotron_setup::pending_download_bytes(
                        &roots.nemotron,
                        self.nemotron_backend(),
                    ),
                    Step::Voice => speech_setup::pending_download_bytes(&roots.voice),
                };
                (step, bytes)
            })
            .collect();
        Some(Plan { steps })
    }

    /// The plan for the screen, measured again only when it can have changed.
    pub(super) fn download_plan(&mut self) -> Option<Plan> {
        let key = self.plan_key()?;
        if let Some((known, plan)) = &self.setup.plan
            && *known == key
        {
            return Some(plan.clone());
        }
        let plan = self.plan_under(&Roots::on_this_pc(self.model_store_root()))?;
        self.setup.plan = Some((key, plan.clone()));
        Some(plan)
    }

    /// Measure the plan under `roots` instead of on this PC, for previews and tests.
    pub(super) fn seed_plan(&mut self, roots: &Roots) {
        if let (Some(key), Some(plan)) = (self.plan_key(), self.plan_under(roots)) {
            self.setup.plan = Some((key, plan));
        }
    }

    /// A stopped chain no longer matches what the screen asks for once the choice changes.
    pub(super) fn forget_stopped_chain(&mut self) {
        if self.setup.chain.as_ref().is_some_and(|c| !c.is_running()) {
            self.setup.chain = None;
        }
    }

    /// A model that does not fit never starts anything.
    pub(super) fn start_chain(&mut self) {
        if self.setup.chain.is_some() || !self.setup_model_fits() {
            return;
        }
        let Some(plan) = self.download_plan() else {
            return;
        };
        self.setup.chain = Some(Chain::new(plan.steps()));
    }

    pub(super) fn retry_chain(&mut self) {
        if let Some(chain) = &mut self.setup.chain {
            chain.retry();
        }
    }

    /// 中止: ask the running step to stop. It answers with its own event.
    pub(super) fn cancel_chain(&mut self) {
        if let Some(step) = self.setup.chain.as_ref().and_then(Chain::current) {
            self.step_cancel(step).store(true, Ordering::Relaxed);
        }
    }

    pub(super) fn step_cancel(&self, step: Step) -> &AtomicBool {
        match step {
            Step::Translation => &self.setup.cancel,
            Step::Punctuation => &self.paragraph_tools.cancel,
            Step::Speech => &self.audio_cancel,
            Step::Voice if self.readout.voice_preparing => &self.readout.voice_cancel,
            Step::Voice => &self.readout.cancel,
        }
    }

    pub(super) fn step_progress(&self, step: Step) -> Option<&Progress> {
        match step {
            Step::Translation => self.setup.progress.as_ref(),
            Step::Punctuation => self.paragraph_tools.progress.as_ref(),
            Step::Speech => self.audio_progress.as_ref(),
            Step::Voice => self.readout.progress.as_ref(),
        }
    }

    /// The chain runs this step, so the step's own dialog stays closed.
    pub(super) fn chain_drives(&self, step: Step) -> bool {
        self.setup
            .chain
            .as_ref()
            .is_some_and(|chain| chain.waits_for(step))
    }

    /// Start every step that is due. Called from the frame loop only.
    pub(super) fn run_chain(&mut self) {
        while let Some(step) = self.setup.chain.as_mut().and_then(Chain::take_next) {
            self.launch(step);
        }
    }

    fn launch(&mut self, step: Step) {
        match step {
            Step::Translation => {
                if self.translation_is_current() {
                    self.finish_step(step);
                } else {
                    self.begin_install();
                }
            }
            Step::Punctuation => {
                if self.paragraph_tools.engine.is_some() {
                    self.finish_step(step);
                } else {
                    self.prepare_paragraph_tools();
                }
            }
            Step::Speech => {
                if self.settings.asr_backend != "nemotron" {
                    self.finish_step(step);
                } else if self.audio_environment_ready() {
                    self.start_asr();
                    self.finish_step(step);
                } else {
                    self.begin_audio_install();
                }
            }
            Step::Voice => self.continue_voice(),
        }
    }

    /// The model in use is the one chosen, on the runtime chosen, and the server has it.
    pub(super) fn translation_is_current(&self) -> bool {
        let (Some(in_use), Some(chosen), Some(recommendation)) = (
            &self.setup.in_use,
            self.setup_model(),
            &self.setup.recommendation,
        ) else {
            return false;
        };
        in_use.id == chosen.id
            && self.settings.backend == recommendation.backend.as_str()
            && Path::new(&self.settings.llama_server).is_file()
            && (self.backend_ready || self.backend_loading)
    }

    /// The readout voice is the runtime and model, then the fixed voice made from them.
    pub(super) fn continue_voice(&mut self) {
        if self.readout.ready.is_none() {
            self.begin_readout_install();
        } else if !self
            .readout
            .voices
            .iter()
            .any(|(key, _)| *key == self.settings.tts_voice_profile)
        {
            self.prepare_readout_voice(None);
        } else {
            self.finish_step(Step::Voice);
        }
    }

    pub(super) fn finish_step(&mut self, step: Step) {
        let Some(chain) = &mut self.setup.chain else {
            return;
        };
        if !chain.complete(step) {
            return;
        }
        self.setup.plan = None;
        if chain.is_finished() {
            self.finish_chain();
        }
    }

    /// An installer stopped. `cancelled` is the step's own cancel flag, as it was when the installer answered.
    pub(super) fn stop_step(&mut self, step: Step, cancelled: bool, error: String) {
        let Some(chain) = &mut self.setup.chain else {
            return;
        };
        let stopped = if cancelled {
            chain.cancel(step)
        } else {
            chain.fail(step, error)
        };
        if stopped {
            self.setup.plan = None;
        }
    }

    /// Everything is prepared. Listening starts only when the user starts it.
    fn finish_chain(&mut self) {
        self.setup.chain = None;
        self.setup.plan = None;
        self.refresh_store_state();
        self.status = READY_STATUS.into();
        if self.tab == Tab::Setup {
            self.tab = Tab::Live;
        }
    }
}

#[cfg(test)]
mod tests;
