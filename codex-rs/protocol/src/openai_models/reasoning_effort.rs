//! Model-owned effort normalization shared by native requests and configuration update items.
//!
//! UI aliases resolve through the same model defaults and fallbacks in both paths.

use super::ModelInfo;
use super::ReasoningEffort;

fn reasoning_effort_rank(effort: &ReasoningEffort) -> Option<u8> {
    match effort {
        ReasoningEffort::None => Some(0),
        ReasoningEffort::Minimal => Some(1),
        ReasoningEffort::Low => Some(2),
        ReasoningEffort::Medium => Some(3),
        ReasoningEffort::High => Some(4),
        ReasoningEffort::XHigh => Some(5),
        ReasoningEffort::Max => Some(6),
        ReasoningEffort::Ultra => Some(7),
        ReasoningEffort::Persistent | ReasoningEffort::Custom(_) => None,
    }
}

impl ModelInfo {
    /// Resolves a selected effort to the value sent in an ordinary inference request.
    pub fn resolve_reasoning_effort(&self, effort: ReasoningEffort) -> ReasoningEffort {
        match effort {
            ReasoningEffort::Ultra => self
                .multi_agent_reasoning_effort
                .as_ref()
                .filter(|effort| {
                    *effort != &ReasoningEffort::Ultra
                        && self
                            .supported_reasoning_levels
                            .iter()
                            .any(|preset| &preset.effort == *effort)
                })
                .cloned()
                .or_else(|| {
                    self.supported_reasoning_levels
                        .iter()
                        .find(|preset| preset.effort == ReasoningEffort::Max)
                        .or_else(|| {
                            self.supported_reasoning_levels
                                .iter()
                                .rev()
                                .find(|preset| preset.effort != ReasoningEffort::Ultra)
                        })
                        .map(|preset| preset.effort.clone())
                })
                .unwrap_or(ReasoningEffort::Medium),
            // Keep "persistent" in local settings; the Responses API calls it "disabled".
            ReasoningEffort::Persistent => ReasoningEffort::Custom("disabled".to_string()),
            effort => effort,
        }
    }

    /// Resolves the reasoning effort to retain when switching a turn or session to this model.
    pub fn resolve_reasoning_effort_for_model_switch(
        &self,
        current_effort: Option<&ReasoningEffort>,
    ) -> Option<ReasoningEffort> {
        if let Some(current_effort) = current_effort {
            if self
                .supported_reasoning_levels
                .iter()
                .any(|preset| &preset.effort == current_effort)
            {
                return Some(current_effort.clone());
            }
            if *current_effort == ReasoningEffort::Ultra {
                return (!self.supported_reasoning_levels.is_empty())
                    .then(|| self.resolve_reasoning_effort(ReasoningEffort::Ultra))
                    .or_else(|| self.default_reasoning_level.clone());
            }
            if let Some(target_rank) = reasoning_effort_rank(current_effort)
                && let Some(clamped) = self
                    .supported_reasoning_levels
                    .iter()
                    .filter(|preset| preset.effort != ReasoningEffort::Ultra)
                    .filter_map(|preset| {
                        let rank = reasoning_effort_rank(&preset.effort)?;
                        (rank <= target_rank).then_some((rank, &preset.effort))
                    })
                    .max_by_key(|(rank, _)| *rank)
                    .map(|(_, effort)| effort.clone())
            {
                return Some(clamped);
            }
        }

        self.default_reasoning_level
            .as_ref()
            .filter(|default_effort| {
                self.supported_reasoning_levels.is_empty()
                    || self
                        .supported_reasoning_levels
                        .iter()
                        .any(|preset| &preset.effort == *default_effort)
            })
            .cloned()
            .or_else(|| {
                self.supported_reasoning_levels
                    .get(self.supported_reasoning_levels.len().saturating_sub(1) / 2)
                    .map(|preset| preset.effort.clone())
            })
    }
}
#[cfg(test)]
#[path = "reasoning_effort_tests.rs"]
mod tests;
