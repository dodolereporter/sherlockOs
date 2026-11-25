//! Interface du fournisseur de modèle (ADR 0012).
//!
//! Un fournisseur reçoit les instructions système, l'historique et la liste des outils, et rend
//! soit un texte, soit des appels d'outils. Le fournisseur cloud et, plus tard, le modèle local
//! implémentent le même trait : l'agent ne sait pas lequel il appelle.

use std::fmt;
use std::future::Future;

use serde_json::Value;

use crate::mcp::ToolSpec;

/// Un message de l'historique de la conversation.
///
/// Les instructions système n'y figurent pas : elles sont fixées à la création de l'agent et
/// passent à part dans [`ModelRequest::system`]. Un résultat d'outil est toujours un
/// [`Message::ToolResult`], que l'adaptateur d'une API rend comme résultat d'outil, jamais comme
/// consigne.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// Demande de l'utilisateur.
    User(String),
    /// Tour du modèle : un texte, ou des appels d'outils.
    Assistant(ModelReply),
    /// Résultat d'un appel d'outil. Une donnée, jamais une consigne.
    ToolResult {
        /// Identifiant de l'appel auquel il répond.
        call_id: String,
        /// Texte rendu par l'outil, ou raison du refus.
        content: String,
        /// Vrai si l'appel a échoué ou a été refusé.
        is_error: bool,
    },
}

/// Un appel d'outil demandé par le modèle.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// Identifiant choisi par le modèle, repris dans le résultat.
    pub id: String,
    /// Nom de l'outil.
    pub name: String,
    /// Arguments, un objet JSON conforme au schéma de l'outil.
    pub arguments: Value,
}

/// Réponse du modèle à une requête.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelReply {
    /// Réponse finale à l'utilisateur.
    Text(String),
    /// Outils à appeler avant de répondre.
    ToolCalls(Vec<ToolCall>),
}

/// Ce que l'agent envoie au modèle à chaque tour.
#[derive(Debug, Clone, Copy)]
pub struct ModelRequest<'a> {
    /// Instructions système, identiques à chaque tour.
    pub system: &'a str,
    /// Historique complet de la conversation.
    pub messages: &'a [Message],
    /// Outils découverts sur les serveurs MCP, avec leur schéma.
    pub tools: &'a [ToolSpec],
}

/// Erreur d'un fournisseur de modèle.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// Le scénario du faux fournisseur n'a plus de réponse.
    #[error("le scénario du faux modèle est épuisé")]
    ScriptEnded,
    /// Le fournisseur ne répond pas ou refuse la requête.
    #[error("fournisseur indisponible : {0}")]
    Unavailable(String),
}

/// Un fournisseur de modèle de langage.
pub trait ModelProvider {
    /// Rend la réponse du modèle à la requête.
    fn complete(
        &mut self,
        request: ModelRequest<'_>,
    ) -> impl Future<Output = Result<ModelReply, ProviderError>> + Send;
}

/// Clé d'API d'un fournisseur cloud. Son contenu ne s'affiche jamais.
pub struct ApiKey(String);

impl ApiKey {
    /// Enveloppe une clé lue dans le trousseau.
    pub fn new(secret: String) -> Self {
        Self(secret)
    }

    /// La clé en clair, pour l'en-tête de la requête au fournisseur.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(masquée)")
    }
}

/// Emplacement de la clé : le trousseau de l'utilisateur, jamais le dépôt ni un fichier de
/// configuration (ADR 0012).
///
/// Aucune implémentation dans le lot A-10 : le fournisseur cloud n'est pas choisi. Le futur
/// fournisseur recevra un `Keyring` à sa création et lira sa clé à chaque connexion.
pub trait Keyring {
    /// Lit la clé du fournisseur nommé.
    fn api_key(&self, provider: &str) -> Result<ApiKey, KeyringError>;
}

/// Erreur de lecture du trousseau.
#[derive(Debug, thiserror::Error)]
pub enum KeyringError {
    /// Aucune clé enregistrée pour ce fournisseur.
    #[error("aucune clé pour le fournisseur {0} dans le trousseau")]
    Missing(String),
    /// Le trousseau est verrouillé ou injoignable.
    #[error("trousseau indisponible : {0}")]
    Unavailable(String),
}
