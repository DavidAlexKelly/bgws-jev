//! A validated set of templates, resolved once, with the vocabulary they were
//! checked against (RUST_ENGINE.md §10.1). The registry half of
//! `behaviour/actions/land.ts` (`behaviourRegistry`, `resolvedTemplate`,
//! `templateChain`) and the template choice of `world/questions.ts`
//! (`templateForTrigger`).

use crate::archetypes::{DEFAULT_TEMPLATE_ID, WORLD_DEFAULT_TEMPLATE};
use crate::facts::Facts;
use crate::gates::Gates;
use crate::load::{build_registry, builtin_files, ActionCompatibility, LoadError, LoadMode, TemplateFiles};
use crate::types::{ResolvedTemplate, Template, WalkResult};
use crate::vocabulary::Vocabulary;
use crate::walk::{resolve_template, walk_gated, Masks, WalkError};
use std::collections::BTreeMap;

/// Templates by id, each resolved against its parents.
#[derive(Clone, Debug)]
pub struct Registry {
    templates: BTreeMap<String, Template>,
    resolved: BTreeMap<String, ResolvedTemplate>,
    vocabulary: Vocabulary,
    compatibility: ActionCompatibility,
}

impl Registry {
    /// Built by the loader after validation, so every template resolves.
    pub(crate) fn from_validated(templates: BTreeMap<String, Template>, vocabulary: Vocabulary) -> Registry {
        let resolved = templates.keys().filter_map(|id| resolve_template(id, &templates).ok().map(|r| (id.clone(), r))).collect();
        let compatibility = ActionCompatibility::of(&templates, &vocabulary);
        Registry { templates, resolved, vocabulary, compatibility }
    }

    /// The shipped templates, checked strictly against the full vocabulary.
    pub fn builtin() -> Result<Registry, LoadError> {
        build_registry(&builtin_files(), &Vocabulary::builtin(), LoadMode::Strict)
    }

    /// The shipped templates plus user ones (a game's `behaviourTemplates`).
    pub fn with_custom(custom: &[Template], vocab: &Vocabulary, mode: LoadMode) -> Result<Registry, LoadError> {
        build_registry(&files_with(custom), vocab, mode)
    }

    /// As `with_custom`, but an invalid user set is ignored (shipped only), as
    /// the TypeScript engine does: a bad template can cost units their custom
    /// behaviour, never break a game. The error says why.
    pub fn with_custom_or_builtin(custom: &[Template], vocab: &Vocabulary, mode: LoadMode) -> Result<(Registry, Option<LoadError>), LoadError> {
        match Self::with_custom(custom, vocab, mode) {
            Ok(r) => Ok((r, None)),
            Err(e) if !custom.is_empty() => Ok((build_registry(&builtin_files(), vocab, mode)?, Some(e))),
            Err(e) => Err(e),
        }
    }

    pub fn templates(&self) -> &BTreeMap<String, Template> {
        &self.templates
    }
    pub fn get(&self, id: &str) -> Option<&Template> {
        self.templates.get(id)
    }
    pub fn contains(&self, id: &str) -> bool {
        self.templates.contains_key(id)
    }
    pub fn vocabulary(&self) -> &Vocabulary {
        &self.vocabulary
    }
    /// Which named actions this engine lacks (§20).
    pub fn compatibility(&self) -> &ActionCompatibility {
        &self.compatibility
    }

    /// A resolved template.
    pub fn resolved(&self, id: &str) -> Option<&ResolvedTemplate> {
        self.resolved.get(id)
    }

    /// A resolved template; an id the registry does not know falls back to core/default.
    pub fn resolved_or_default(&self, id: &str) -> Option<&ResolvedTemplate> {
        self.resolved.get(id).or_else(|| self.resolved.get(DEFAULT_TEMPLATE_ID))
    }

    /// The ids of a template and its ancestors, itself first.
    pub fn lineage(&self, id: &str) -> Vec<String> {
        self.resolved_or_default(id).map(|r| r.lineage.clone()).unwrap_or_default()
    }

    /// The template that answers this trigger for an agent (`templateForTrigger`).
    ///
    /// `candidates` are the agent's template ids, most specific first (its
    /// assignment, its entity's template, its archetype's). The first that
    /// exists and has the trigger answers. If none has it (a land template
    /// knows only D0–D12), core/world's node answers, with the agent's first
    /// template's doctrine, removals, biases and conditions applied: a variant
    /// that says "never pull back" still means it when the question comes
    /// from a world system.
    pub fn template_for_trigger(&self, candidates: &[&str], trigger: &str) -> Option<ResolvedTemplate> {
        let own: Vec<&ResolvedTemplate> = candidates.iter().filter_map(|id| self.resolved.get(*id)).collect();
        if let Some(answering) = own.iter().find(|t| t.triggers.contains_key(trigger)) {
            return Some((*answering).clone());
        }
        let world = self.resolved.get(WORLD_DEFAULT_TEMPLATE)?;
        if !world.triggers.contains_key(trigger) {
            return None;
        }
        let Some(mine) = own.first() else { return Some(world.clone()) };
        let mut merged = world.clone();
        merged.id = mine.id.clone();
        merged.lineage = mine.lineage.iter().cloned().chain(world.lineage.iter().filter(|id| !mine.lineage.contains(id)).cloned()).collect();
        merged.doctrine = world.doctrine.iter().cloned().chain(mine.doctrine.iter().filter(|d| !world.doctrine.contains(d)).cloned()).collect();
        merged.remove.extend(mine.remove.iter().cloned());
        merged.bias.extend(mine.bias.iter().map(|(k, v)| (k.clone(), *v)));
        merged.when.extend(mine.when.iter().map(|(k, v)| (k.clone(), v.clone())));
        Some(merged)
    }

    /// Walk a resolved template with the hard gates and this registry's vocabulary.
    pub fn walk(&self, template: &ResolvedTemplate, trigger: &str, facts: &impl Facts, gates: &Gates) -> Result<WalkResult, WalkError> {
        walk_gated(template, trigger, facts, Masks { gates, vocabulary: &self.vocabulary })
    }

    /// Choose the template for an agent's trigger and walk it, gated. `None`
    /// when no template has anything for this trigger.
    pub fn decide(&self, candidates: &[&str], trigger: &str, facts: &impl Facts, gates: &Gates) -> Result<Option<(ResolvedTemplate, WalkResult)>, WalkError> {
        let Some(t) = self.template_for_trigger(candidates, trigger) else { return Ok(None) };
        let r = self.walk(&t, trigger, facts, gates)?;
        Ok(Some((t, r)))
    }
}

/// The shipped files plus these user templates, keyed as if on disk.
pub fn files_with(custom: &[Template]) -> TemplateFiles {
    let mut files = builtin_files();
    for t in custom {
        files.insert(format!("./templates/{}.json", t.id), serde_json::to_value(t).unwrap_or_default());
    }
    files
}
