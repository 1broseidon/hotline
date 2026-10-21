//! Model data read once for one listing; nothing survives the request.
use super::*;

pub(super) struct ModelSources {
    pub(super) connections: HashMap<String, (Credential, ProviderAuth)>,
    pub(super) discovered: HashMap<String, Option<Vec<ListedModel>>>,
    pub(super) manual: HashMap<String, Vec<String>>,
}

impl ModelSources {
    pub(super) fn account_models(&self) -> HashMap<String, Vec<String>> {
        self.connections
            .iter()
            .filter_map(|(provider_id, (credential, _))| {
                if let Some(custom) = &credential.custom {
                    return Some((provider_id.clone(), custom.models.clone()));
                }
                let discovered = self.discovered.get(&credential.id).cloned().flatten();
                let manual = self.manual.get(&credential.id).cloned().unwrap_or_default();
                let mut ids = match discovered {
                    Some(models) => models.into_iter().map(|model| model.id).collect::<Vec<_>>(),
                    None if manual.is_empty() => return None,
                    None => crate::models::catalog()
                        .providers
                        .get(provider_id)
                        .map(|entry| entry.models.keys().cloned().collect())
                        .unwrap_or_default(),
                };
                // Copilot's successful account response remains authoritative,
                // including when an older manual addition is no longer offered.
                if provider_id != "github-copilot" {
                    ids.extend(manual);
                }
                ids.sort();
                ids.dedup();
                Some((provider_id.clone(), ids))
            })
            .collect()
    }

    /// The same resolved metadata feeds Settings, pickers, and request limits.
    pub(crate) fn model_metadata(&self) -> HashMap<String, crate::contract::CatalogModel> {
        let mut result = HashMap::new();
        for (provider_id, (credential, _)) in &self.connections {
            let discovered: BTreeMap<_, _> = self
                .discovered
                .get(&credential.id)
                .cloned()
                .flatten()
                .unwrap_or_default()
                .into_iter()
                .map(|model| (model.id.clone(), model))
                .collect();
            let manual: BTreeSet<_> = self
                .manual
                .get(&credential.id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect();
            let ids: Vec<_> = discovered
                .keys()
                .chain(manual.iter())
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            for mut model in crate::models::catalog_models(provider_id, &HashMap::new(), Some(&ids))
            {
                model.manual = manual.contains(&model.id);
                if let Some(listed) = discovered.get(&model.id) {
                    if let Some(name) = &listed.name {
                        model.name.clone_from(name);
                    }
                    model.context_limit = listed.context_limit.or(model.context_limit);
                    model.output_limit = listed.output_limit.or(model.output_limit);
                }
                result.insert(format!("{provider_id}/{}", model.id), model);
            }
        }
        result
    }
}
