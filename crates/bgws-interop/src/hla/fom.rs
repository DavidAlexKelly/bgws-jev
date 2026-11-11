//! A federation's object model, read from its FOM modules (the HLA OMT XML,
//! IEEE 1516.2): the object classes with their attributes and the
//! interaction classes with their parameters, each in its class tree. The
//! RTI ([`super::mock`]) uses it to give out handles, resolve names, and
//! decide who is told what.
//!
//! - Modules merge: a class named in several adds what each declares.
//! - Names resolve in full (`HLAobjectRoot.BaseEntity.PhysicalEntity`),
//!   without the root (`BaseEntity.PhysicalEntity`), or by a unique last
//!   part or tail (`PhysicalEntity`).
//! - An attribute belongs to the class that declares it; its subclasses
//!   inherit it under the same handle. The same for parameters.
//! - The MIM's management classes (`HLAmanager.HLAfederation`,
//!   `HLAmanager.HLAfederate`) are always there.
//! - A federation created with no module is *permissive*: any name asked
//!   for is added (with the classes on its path), so a federation of BGWS
//!   federates needs no FOM file.

use std::collections::BTreeMap;

pub const OBJECT_ROOT: &str = "HLAobjectRoot";
pub const INTERACTION_ROOT: &str = "HLAinteractionRoot";
pub const FEDERATION: &str = "HLAobjectRoot.HLAmanager.HLAfederation";
pub const FEDERATE: &str = "HLAobjectRoot.HLAmanager.HLAfederate";

/// The MIM attributes the RTI keeps up to date (a subset of IEEE 1516.1's).
pub const FEDERATION_ATTRIBUTES: [&str; 3] = ["HLAfederationName", "HLAfederatesInFederation", "HLAtimeImplementationName"];
pub const FEDERATE_ATTRIBUTES: [&str; 7] = ["HLAfederateHandle", "HLAfederateName", "HLAfederateType", "HLAtimeRegulating", "HLAtimeConstrained", "HLAlogicalTime", "HLAlookahead"];

#[derive(Clone, Debug, PartialEq)]
pub struct Class {
    /// Full name, from the root.
    pub name: String,
    pub parent: Option<usize>,
    /// Attributes (or parameters) declared here: indices into the members.
    pub members: Vec<usize>,
}

/// One tree: object classes and attributes, or interaction classes and parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct Tree {
    pub classes: Vec<Class>,
    /// (declaring class, name).
    pub members: Vec<(usize, String)>,
    by_name: BTreeMap<String, usize>,
}

impl Tree {
    fn new(root: &str) -> Tree {
        let mut by_name = BTreeMap::new();
        by_name.insert(root.to_string(), 0);
        Tree { classes: vec![Class { name: root.into(), parent: None, members: vec![] }], members: vec![], by_name }
    }

    fn root(&self) -> &str {
        &self.classes[0].name
    }

    /// The class of a full name, adding it (and its path) if missing.
    fn add_class(&mut self, full: &str) -> usize {
        if let Some(&c) = self.by_name.get(full) {
            return c;
        }
        let parent = match full.rfind('.') {
            Some(i) => self.add_class(&full[..i]),
            None => 0,
        };
        let c = self.classes.len();
        self.classes.push(Class { name: full.into(), parent: Some(parent), members: vec![] });
        self.by_name.insert(full.into(), c);
        c
    }

    fn add_member(&mut self, class: usize, name: &str) -> usize {
        if let Some(m) = self.member_at(class, name) {
            return m;
        }
        let m = self.members.len();
        self.members.push((class, name.into()));
        self.classes[class].members.push(m);
        m
    }

    /// The class a name means: in full, without the root, or by a unique tail.
    pub fn class(&self, name: &str) -> Option<usize> {
        let root = self.root();
        let full = if name == root || name.starts_with(&format!("{root}.")) { name.to_string() } else { format!("{root}.{name}") };
        if let Some(&c) = self.by_name.get(&full) {
            return Some(c);
        }
        let tail = format!(".{name}");
        let mut found = self.by_name.iter().filter(|(k, _)| k.ends_with(&tail)).map(|(_, &c)| c);
        match (found.next(), found.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        }
    }

    /// This class and each of its superclasses, nearest first.
    pub fn lineage(&self, mut c: usize) -> Vec<usize> {
        let mut out = vec![c];
        while let Some(p) = self.classes[c].parent {
            out.push(p);
            c = p;
        }
        out
    }

    pub fn is_a(&self, c: usize, of: usize) -> bool {
        self.lineage(c).contains(&of)
    }

    /// An attribute (parameter) a class has, its own or inherited.
    pub fn member_at(&self, class: usize, name: &str) -> Option<usize> {
        self.lineage(class).into_iter().find_map(|c| self.classes[c].members.iter().copied().find(|&m| self.members[m].1 == name))
    }

    /// Does the class have this member (its own or inherited)?
    pub fn has_member(&self, class: usize, m: usize) -> bool {
        self.members.get(m).is_some_and(|(owner, _)| self.is_a(class, *owner))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fom {
    pub objects: Tree,
    pub interactions: Tree,
    /// Names asked for that are not in it are added.
    pub permissive: bool,
    /// The modules merged, by name.
    pub modules: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FomError(pub String);

impl std::fmt::Display for FomError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A module as read: (class path, members) for objects and for interactions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Module {
    pub name: String,
    pub objects: Vec<(Vec<String>, Vec<String>)>,
    pub interactions: Vec<(Vec<String>, Vec<String>)>,
}

fn child<'a>(n: roxmltree::Node<'a, 'a>, tag: &str) -> Option<roxmltree::Node<'a, 'a>> {
    n.children().find(|c| c.is_element() && c.tag_name().name() == tag)
}

fn text_of(n: roxmltree::Node, tag: &str) -> Option<String> {
    child(n, tag).and_then(|c| c.text()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

fn walk(n: roxmltree::Node, class_tag: &str, member_tag: &str, path: &mut Vec<String>, out: &mut Vec<(Vec<String>, Vec<String>)>) -> Result<(), FomError> {
    let name = text_of(n, "name").ok_or_else(|| FomError(format!("a <{class_tag}> with no name under {}", path.join("."))))?;
    path.push(name);
    let members = n.children().filter(|c| c.is_element() && c.tag_name().name() == member_tag).filter_map(|c| text_of(c, "name")).collect();
    out.push((path.clone(), members));
    for c in n.children().filter(|c| c.is_element() && c.tag_name().name() == class_tag) {
        walk(c, class_tag, member_tag, path, out)?;
    }
    path.pop();
    Ok(())
}

impl Module {
    /// Read a FOM module (OMT XML, any of the 1516 versions' namespaces).
    pub fn parse(name: &str, xml: &str) -> Result<Module, FomError> {
        let doc = roxmltree::Document::parse(xml).map_err(|e| FomError(format!("{name}: not XML: {e}")))?;
        let root = doc.root_element();
        if root.tag_name().name() != "objectModel" {
            return Err(FomError(format!("{name}: not an object model (root <{}>)", root.tag_name().name())));
        }
        let mut m = Module { name: name.into(), ..Default::default() };
        if let Some(objs) = child(root, "objects") {
            for c in objs.children().filter(|c| c.is_element() && c.tag_name().name() == "objectClass") {
                walk(c, "objectClass", "attribute", &mut vec![], &mut m.objects)?;
            }
        }
        if let Some(ints) = child(root, "interactions") {
            for c in ints.children().filter(|c| c.is_element() && c.tag_name().name() == "interactionClass") {
                walk(c, "interactionClass", "parameter", &mut vec![], &mut m.interactions)?;
            }
        }
        for (path, _) in &m.objects {
            if path[0] != OBJECT_ROOT {
                return Err(FomError(format!("{name}: object class {} is not under {OBJECT_ROOT}", path.join("."))));
            }
        }
        for (path, _) in &m.interactions {
            if path[0] != INTERACTION_ROOT {
                return Err(FomError(format!("{name}: interaction class {} is not under {INTERACTION_ROOT}", path.join("."))));
            }
        }
        Ok(m)
    }
}

impl Fom {
    /// The MIM alone (a federation with no FOM module given is permissive).
    pub fn new(permissive: bool) -> Fom {
        let mut f = Fom { objects: Tree::new(OBJECT_ROOT), interactions: Tree::new(INTERACTION_ROOT), permissive, modules: vec![] };
        f.objects.add_member(0, "HLAprivilegeToDeleteObject");
        let fx = f.objects.add_class(FEDERATION);
        for a in FEDERATION_ATTRIBUTES {
            f.objects.add_member(fx, a);
        }
        let fd = f.objects.add_class(FEDERATE);
        for a in FEDERATE_ATTRIBUTES {
            f.objects.add_member(fd, a);
        }
        f
    }

    /// From the modules a federation is created with.
    pub fn from_modules(modules: Vec<Module>) -> Result<Fom, FomError> {
        let mut f = Fom::new(modules.is_empty());
        for m in modules {
            f.merge(m)?;
        }
        Ok(f)
    }

    /// Add a module's classes and members.
    pub fn merge(&mut self, m: Module) -> Result<(), FomError> {
        for (path, members) in &m.objects {
            let c = self.objects.add_class(&path.join("."));
            for a in members {
                self.objects.add_member(c, a);
            }
        }
        for (path, members) in &m.interactions {
            let c = self.interactions.add_class(&path.join("."));
            for p in members {
                self.interactions.add_member(c, p);
            }
        }
        self.modules.push(m.name);
        Ok(())
    }

    /// An object class by name (added if permissive).
    pub fn object_class(&mut self, name: &str) -> Option<usize> {
        self.objects.class(name).or_else(|| self.permissive.then(|| self.objects.add_class(&full(OBJECT_ROOT, name))))
    }

    pub fn interaction_class(&mut self, name: &str) -> Option<usize> {
        self.interactions.class(name).or_else(|| self.permissive.then(|| self.interactions.add_class(&full(INTERACTION_ROOT, name))))
    }

    /// An attribute of a class by name (declared there if permissive and new).
    pub fn attribute(&mut self, class: usize, name: &str) -> Option<usize> {
        self.objects.member_at(class, name).or_else(|| self.permissive.then(|| self.objects.add_member(class, name)))
    }

    pub fn parameter(&mut self, class: usize, name: &str) -> Option<usize> {
        self.interactions.member_at(class, name).or_else(|| self.permissive.then(|| self.interactions.add_member(class, name)))
    }
}

impl Default for Fom {
    fn default() -> Self {
        Fom::new(true)
    }
}

fn full(root: &str, name: &str) -> String {
    if name == root || name.starts_with(&format!("{root}.")) {
        name.to_string()
    } else {
        format!("{root}.{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"<?xml version="1.0"?>
<objectModel xmlns="http://standards.ieee.org/IEEE1516-2025">
  <objects><objectClass><name>HLAobjectRoot</name>
    <objectClass><name>BaseEntity</name>
      <attribute><name>EntityType</name></attribute>
      <attribute><name>Spatial</name></attribute>
      <objectClass><name>PhysicalEntity</name>
        <attribute><name>DamageState</name></attribute>
        <objectClass><name>Platform</name>
          <objectClass><name>GroundVehicle</name></objectClass>
        </objectClass>
      </objectClass>
    </objectClass>
  </objectClass></objects>
  <interactions><interactionClass><name>HLAinteractionRoot</name>
    <interactionClass><name>WeaponFire</name><parameter><name>EventIdentifier</name></parameter></interactionClass>
  </interactionClass></interactions>
</objectModel>"#;

    const EXTRA: &str = r#"<objectModel xmlns="http://standards.ieee.org/IEEE1516-2010">
  <objects><objectClass><name>HLAobjectRoot</name>
    <objectClass><name>BaseEntity</name>
      <objectClass><name>PhysicalEntity</name>
        <objectClass><name>Platform</name>
          <objectClass><name>Aircraft</name><attribute><name>AfterburnerOn</name></attribute></objectClass>
        </objectClass>
      </objectClass>
    </objectClass>
  </objectClass></objects>
</objectModel>"#;

    #[test]
    fn modules_merge_into_one_class_tree() {
        let mut f = Fom::from_modules(vec![Module::parse("base.xml", BASE).unwrap(), Module::parse("extra.xml", EXTRA).unwrap()]).unwrap();
        assert!(!f.permissive);
        let gv = f.object_class("BaseEntity.PhysicalEntity.Platform.GroundVehicle").unwrap();
        assert_eq!(f.object_class("HLAobjectRoot.BaseEntity.PhysicalEntity.Platform.GroundVehicle"), Some(gv));
        assert_eq!(f.object_class("GroundVehicle"), Some(gv), "a unique last part");
        let air = f.object_class("Aircraft").unwrap();
        let base = f.object_class("BaseEntity").unwrap();
        assert!(f.objects.is_a(gv, base) && f.objects.is_a(air, base) && !f.objects.is_a(base, gv));
        // Inherited attributes keep their declaring class's handle.
        assert_eq!(f.attribute(gv, "EntityType"), f.attribute(base, "EntityType"));
        assert!(f.attribute(base, "DamageState").is_none(), "declared below BaseEntity");
        assert!(f.attribute(air, "AfterburnerOn").is_some() && f.attribute(gv, "AfterburnerOn").is_none());
        assert!(f.object_class("Tank").is_none(), "not in the FOM");
        assert!(f.interaction_class("WeaponFire").is_some());
        // The MIM is always there.
        assert!(f.object_class("HLAmanager.HLAfederate").is_some());
        assert_eq!(f.modules, ["base.xml", "extra.xml"]);
    }

    #[test]
    fn with_no_module_any_name_is_added_with_its_path() {
        let mut f = Fom::from_modules(vec![]).unwrap();
        assert!(f.permissive);
        let gv = f.object_class("HLAobjectRoot.BaseEntity.PhysicalEntity.Platform.GroundVehicle").unwrap();
        let base = f.object_class("BaseEntity").unwrap();
        assert!(f.objects.is_a(gv, base));
        assert!(f.attribute(gv, "Marking").is_some());
    }

    #[test]
    fn bad_modules_are_refused() {
        assert!(Module::parse("x", "<notAModel/>").is_err());
        assert!(Module::parse("x", "not xml").is_err());
        assert!(Module::parse("x", "<objectModel><objects><objectClass><name>Thing</name></objectClass></objects></objectModel>").is_err(), "not under HLAobjectRoot");
    }
}
