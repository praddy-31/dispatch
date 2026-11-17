//! The topic picker's catalogue: friendly groups ("Optics & photonics") over arXiv's real
//! categories (`physics.optics`), with each category's full name. Read-only data.

use serde::{Deserialize, Serialize};

use crate::topics::{Category, ResearchTopic};

pub const JSON: &str = include_str!("arxiv_catalogue.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalogue {
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub categories: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    /// Narrows this category inside the group (becomes `category_include`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub r#match: Vec<String>,
}

pub fn load() -> Catalogue {
    serde_json::from_str(JSON).expect("arxiv_catalogue.json is valid")
}

impl Catalogue {
    /// A new topic from a group, every category switched on (then fine-tuned in the picker).
    pub fn topic_from(&self, group_id: &str) -> Option<ResearchTopic> {
        let g = self.groups.iter().find(|g| g.id == group_id)?;
        Some(ResearchTopic {
            id: g.id.clone(),
            name: g.name.clone(),
            enabled: true,
            categories: g
                .categories
                .iter()
                .map(|c| Category {
                    id: c.id.clone(),
                    enabled: true,
                })
                .collect(),
            include: vec![],
            exclude: vec![],
            category_include: g
                .categories
                .iter()
                .filter(|c| !c.r#match.is_empty())
                .map(|c| (c.id.clone(), c.r#match.clone()))
                .collect(),
        })
    }

    /// The full name of a category id, if the catalogue knows it.
    pub fn name_of(&self, id: &str) -> Option<&str> {
        self.groups
            .iter()
            .flat_map(|g| &g.categories)
            .find(|c| c.id == id)
            .map(|c| c.name.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topics::is_category_id;

    #[test]
    fn catalogue_is_valid_and_every_id_looks_right() {
        let c = load();
        assert!(c.groups.len() > 15);
        for g in &c.groups {
            assert!(!g.categories.is_empty(), "{}", g.id);
            for e in &g.categories {
                assert!(is_category_id(&e.id), "{}", e.id);
            }
        }
        assert_eq!(c.name_of("physics.optics"), Some("Optics"));
    }

    #[test]
    fn a_group_becomes_a_topic_with_its_narrowing_words() {
        let t = load().topic_from("quantum-computing").unwrap();
        assert!(t.categories.iter().all(|c| c.enabled));
        assert!(t.category_include["quant-ph"].iter().any(|w| w == "qubit"));
        assert!(!t.category_include.contains_key("cs.ET"));
    }
}
