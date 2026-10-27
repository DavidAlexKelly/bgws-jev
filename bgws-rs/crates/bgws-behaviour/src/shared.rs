//! Templates shared across a team (DECISION_FLOW_GRAPHS.md §6, step 9). A
//! port of `behaviour/shared.ts`: one shared object per template (id, name,
//! version, parent and the parent's version, archetype, the JSON body and its
//! author), with the transport handed in as a [`TemplateRemote`]. Nothing
//! here touches the network; tests use [`MemoryRemote`].

use crate::types::Template;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// One template as its shared object holds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedTemplate {
    pub template_id: String,
    pub name: String,
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archetype: Option<String>,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum RemoteError {
    #[error("{0} exists")]
    Exists(String),
    #[error("{0} does not exist")]
    Missing(String),
    #[error("remote: {0}")]
    Other(String),
}

/// Where shared templates live: lists them and applies the actions.
pub trait TemplateRemote {
    fn list(&self) -> Result<Vec<SharedTemplate>, RemoteError>;
    fn create(&mut self, t: SharedTemplate) -> Result<(), RemoteError>;
    fn edit(&mut self, t: SharedTemplate) -> Result<(), RemoteError>;
    fn remove(&mut self, template_id: &str) -> Result<(), RemoteError>;
}

/// A user template as its shared object: the template itself is the body.
pub fn to_shared(t: &Template, all: &[Template], author: Option<&str>) -> SharedTemplate {
    let parent = t.parent.as_ref().and_then(|p| all.iter().find(|x| &x.id == p));
    SharedTemplate {
        template_id: t.id.clone(),
        name: t.name.clone().unwrap_or_else(|| t.id.clone()),
        version: t.version,
        parent_id: t.parent.clone(),
        parent_version: parent.map(|p| p.version),
        archetype: t.archetype.clone(),
        body: serde_json::to_string(t).unwrap_or_default(),
        author: author.map(Into::into),
        updated_at: None,
    }
}

/// Back from the shared object; refuses one whose body is not its template.
pub fn from_shared(s: &SharedTemplate) -> Result<Template, String> {
    let t: Template = serde_json::from_str(&s.body).map_err(|_| format!("{}: the body is not valid JSON", s.template_id))?;
    if t.id != s.template_id {
        return Err(format!("{}: the body is for \"{}\"", s.template_id, t.id));
    }
    Ok(t)
}

#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Conflict {
    pub id: String,
    pub mine: u32,
    pub theirs: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PublishResult {
    pub created: Vec<String>,
    pub edited: Vec<String>,
    /// Someone else's newer version is there: not overwritten.
    pub conflicts: Vec<Conflict>,
}

/// Publish these templates: new ones are created, newer versions replace the
/// shared ones, and a template whose shared copy is newer is left alone and
/// reported (someone else changed it: pull first).
pub fn publish(remote: &mut impl TemplateRemote, templates: &[Template], all: &[Template], author: Option<&str>) -> Result<PublishResult, RemoteError> {
    let shared: BTreeMap<String, SharedTemplate> = remote.list()?.into_iter().map(|s| (s.template_id.clone(), s)).collect();
    let mut out = PublishResult::default();
    for t in templates {
        let mine = to_shared(t, all, author);
        match shared.get(&t.id) {
            None => {
                remote.create(mine)?;
                out.created.push(t.id.clone());
            }
            Some(there) if there.version > t.version => {
                out.conflicts.push(Conflict { id: t.id.clone(), mine: t.version, theirs: there.version, by: there.author.clone() });
            }
            Some(there) if there.version < t.version || there.body != mine.body => {
                let bump = u32::from(there.body != mine.body && there.version == t.version);
                let version = t.version.max(there.version + bump);
                remote.edit(SharedTemplate { version, ..mine })?;
                out.edited.push(t.id.clone());
            }
            Some(_) => {}
        }
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct PullResult {
    /// The templates to keep locally: the newer of each, by version.
    pub templates: Vec<Template>,
    pub added: Vec<String>,
    pub updated: Vec<String>,
    pub problems: Vec<String>,
    /// A shared template built on a version of its parent that is no longer the one shared.
    pub stale_parents: Vec<String>,
}

/// Pull the team's templates and merge them with these local ones: the higher version wins.
pub fn pull(remote: &impl TemplateRemote, local: &[Template]) -> Result<PullResult, RemoteError> {
    let mut by_id: BTreeMap<String, Template> = local.iter().map(|t| (t.id.clone(), t.clone())).collect();
    let mut out = PullResult::default();
    let shared = remote.list()?;
    let versions: BTreeMap<&str, u32> = shared.iter().map(|s| (s.template_id.as_str(), s.version)).collect();
    for s in &shared {
        let t = match from_shared(s) {
            Ok(t) => t,
            Err(p) => {
                out.problems.push(p);
                continue;
            }
        };
        if !t.id.starts_with("custom/") {
            out.problems.push(format!("{}: only custom templates are shared", t.id));
            continue;
        }
        if let (Some(pid), Some(pv)) = (&s.parent_id, s.parent_version) {
            if versions.get(pid.as_str()).is_some_and(|v| *v != pv) {
                out.stale_parents.push(t.id.clone());
            }
        }
        match by_id.get(&t.id) {
            None => {
                out.added.push(t.id.clone());
                by_id.insert(t.id.clone(), t);
            }
            Some(mine) if t.version > mine.version => {
                out.updated.push(t.id.clone());
                by_id.insert(t.id.clone(), t);
            }
            Some(_) => {}
        }
    }
    out.templates = by_id.into_values().collect();
    Ok(out)
}

/// An in-memory remote, for tests and for working offline.
#[derive(Clone, Debug, Default)]
pub struct MemoryRemote {
    /// In creation order (as a JavaScript `Map` iterates).
    pub objects: Vec<SharedTemplate>,
}

impl MemoryRemote {
    pub fn get(&self, id: &str) -> Option<&SharedTemplate> {
        self.objects.iter().find(|s| s.template_id == id)
    }
}

impl TemplateRemote for MemoryRemote {
    fn list(&self) -> Result<Vec<SharedTemplate>, RemoteError> {
        Ok(self.objects.clone())
    }
    fn create(&mut self, t: SharedTemplate) -> Result<(), RemoteError> {
        if self.get(&t.template_id).is_some() {
            return Err(RemoteError::Exists(t.template_id));
        }
        self.objects.push(t);
        Ok(())
    }
    fn edit(&mut self, t: SharedTemplate) -> Result<(), RemoteError> {
        let slot = self.objects.iter_mut().find(|s| s.template_id == t.template_id).ok_or_else(|| RemoteError::Missing(t.template_id.clone()))?;
        *slot = t;
        Ok(())
    }
    fn remove(&mut self, template_id: &str) -> Result<(), RemoteError> {
        self.objects.retain(|s| s.template_id != template_id);
        Ok(())
    }
}
