//! Démon de Sherlock : conversation, appels au modèle, enchaînement des outils.
//!
//! - [`modele`] : l'interface du fournisseur de modèle et l'emplacement de sa clé.
//! - [`faux`] : un faux fournisseur piloté par un scénario.
//! - [`mcp`] : le client MCP sur stdio et le niveau des outils.
//! - [`agent`] : la boucle d'agent et ses règles de sécurité.
//! - [`serveur_essai`] : un serveur MCP de test, sans effet sur le système.
//!
//! Architecture et règles : `docs/sherlockd.md`.

pub mod agent;
pub mod faux;
pub mod mcp;
pub mod modele;
pub mod serveur_essai;
