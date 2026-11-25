//! Faux fournisseur de modèle, piloté par un scénario, pour les tests et `--essai`.

use std::collections::VecDeque;

use crate::modele::{Message, ModelProvider, ModelReply, ModelRequest, ProviderError};

type Step = Box<dyn FnMut(&ModelRequest<'_>) -> ModelReply + Send>;

/// Une requête reçue par le faux modèle, gardée pour que les tests l'inspectent.
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    /// Instructions système reçues.
    pub system: String,
    /// Historique reçu.
    pub messages: Vec<Message>,
    /// Noms des outils proposés.
    pub tools: Vec<String>,
}

/// Rend, tour après tour, les réponses de son scénario.
#[derive(Default)]
pub struct ScriptedProvider {
    steps: VecDeque<Step>,
    /// Toutes les requêtes reçues, dans l'ordre.
    pub requests: Vec<RecordedRequest>,
}

impl ScriptedProvider {
    /// Un scénario vide.
    pub fn new() -> Self {
        Self::default()
    }

    /// Ajoute une réponse fixe.
    pub fn reply(self, reply: ModelReply) -> Self {
        self.then(move |_| reply.clone())
    }

    /// Ajoute une réponse calculée à partir de la requête, par exemple du dernier résultat d'outil.
    pub fn then(
        mut self,
        step: impl FnMut(&ModelRequest<'_>) -> ModelReply + Send + 'static,
    ) -> Self {
        self.steps.push_back(Box::new(step));
        self
    }
}

impl ModelProvider for ScriptedProvider {
    async fn complete(&mut self, request: ModelRequest<'_>) -> Result<ModelReply, ProviderError> {
        self.requests.push(RecordedRequest {
            system: request.system.to_owned(),
            messages: request.messages.to_vec(),
            tools: request.tools.iter().map(|t| t.name.clone()).collect(),
        });
        let mut step = self.steps.pop_front().ok_or(ProviderError::ScriptEnded)?;
        Ok(step(&request))
    }
}

/// Texte du dernier résultat d'outil de l'historique, pour les scénarios qui le reprennent.
pub fn last_tool_result(request: &ModelRequest<'_>) -> Option<String> {
    request.messages.iter().rev().find_map(|m| match m {
        Message::ToolResult { content, .. } => Some(content.clone()),
        _ => None,
    })
}
