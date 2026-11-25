//! Fournisseur Claude Code, sans jamais appeler Claude Code : transcriptions `stream-json`
//! enregistrées (identifiants retirés), faux programmes `claude` pour les erreurs et l'historique.

use serde_json::json;
use sherlockd::claude_code::{
    ClaudeCode, ClaudeCodeConfig, ClaudeCodeError, MAX_HISTORY_BYTES, Transcript, read_events,
};
use sherlockd::mcp::MAX_MESSAGE_BYTES;
use sherlockd::modele::{Message, ModelReply, ToolCall};
use sherlockd::serveur_essai::MARQUEUR_SANS_NIVEAU;

const ECHO: &str = include_str!("transcriptions/echo.jsonl");
const REFUS: &str = include_str!("transcriptions/refus.jsonl");
const INTERDIT: &str = include_str!("transcriptions/interdit.jsonl");
const NON_CONNECTE: &str = include_str!("transcriptions/non-connecte.jsonl");

async fn read(transcript: &str) -> Result<Transcript, ClaudeCodeError> {
    read_events(transcript.as_bytes()).await
}

fn calls(transcript: &Transcript) -> Vec<&ToolCall> {
    let calls = transcript.messages.iter().filter_map(|m| match m {
        Message::Assistant(ModelReply::ToolCalls(calls)) => Some(calls),
        _ => None,
    });
    calls.flatten().collect()
}

#[tokio::test]
async fn echo_relaye_dans_la_conversation() {
    let transcript = read(ECHO).await.unwrap();
    let call = ToolCall {
        id: "toolu_essai_1".into(),
        name: "echo".into(),
        arguments: json!({ "texte": "Bonjour" }),
    };
    assert_eq!(
        transcript.messages[0],
        Message::Assistant(ModelReply::ToolCalls(vec![call]))
    );
    let result = Message::ToolResult {
        call_id: "toolu_essai_1".into(),
        content: "Bonjour".into(),
        is_error: false,
    };
    assert_eq!(transcript.messages[1], result);
    assert!(transcript.answer.unwrap().contains("Bonjour"));
}

#[tokio::test]
async fn refus_du_mandataire_relaye() {
    let transcript = read(REFUS).await.unwrap();
    assert_eq!(calls(&transcript)[0].name, "vider_corbeille");
    let Message::ToolResult {
        content, is_error, ..
    } = &transcript.messages[1]
    else {
        panic!("résultat attendu : {:?}", transcript.messages);
    };
    assert!(is_error);
    assert!(content.contains("courtier d'accords absent"), "{content}");
    assert!(!format!("{transcript:?}").contains(MARQUEUR_SANS_NIVEAU));
}

#[tokio::test]
async fn sans_outil_integre_rien_n_est_lu_ni_lance() {
    // Demande de lire /etc/hostname et de lancer ls : aucun appel, Claude Code n'a pas d'outil.
    let transcript = read(INTERDIT).await.unwrap();
    assert!(calls(&transcript).is_empty());
    assert!(transcript.answer.is_some());
}

#[tokio::test]
async fn non_connecte_explique() {
    let error = read(NON_CONNECTE).await.unwrap_err();
    assert!(matches!(error, ClaudeCodeError::NotLoggedIn), "{error}");
    assert!(error.to_string().contains("/login"));
}

/// La transcription d'echo, avec l'événement init modifié.
fn with_init(edit: impl FnOnce(&mut serde_json::Value)) -> String {
    let mut lines: Vec<String> = ECHO.lines().map(String::from).collect();
    // Hors d'une fonction de test, pas de unwrap : un JSON illisible fait échouer le test plus loin.
    let mut init: serde_json::Value = serde_json::from_str(&lines[0]).unwrap_or_default();
    edit(&mut init);
    lines[0] = init.to_string();
    lines.join("\n") + "\n"
}

#[tokio::test]
async fn outil_integre_ou_autre_serveur_arrete_tout() {
    let bash = with_init(|i| i["tools"].as_array_mut().unwrap().push(json!("Bash")));
    let error = read(&bash).await.unwrap_err();
    assert!(
        matches!(&error, ClaudeCodeError::ForbiddenTools(t) if t == "Bash"),
        "{error}"
    );

    let other = with_init(|i| {
        let server = json!({ "name": "autre", "status": "connected" });
        i["mcp_servers"].as_array_mut().unwrap().push(server);
    });
    let error = read(&other).await.unwrap_err();
    assert!(
        matches!(error, ClaudeCodeError::ForbiddenTools(_)),
        "{error}"
    );

    let failed = with_init(|i| i["mcp_servers"][0]["status"] = json!("failed"));
    let error = read(&failed).await.unwrap_err();
    assert!(
        matches!(error, ClaudeCodeError::ProxyNotConnected(_)),
        "{error}"
    );

    let unprefixed = ECHO.replace("mcp__sherlock__echo\", \"input", "Bash\", \"input");
    let error = read(&unprefixed).await.unwrap_err();
    assert!(
        matches!(error, ClaudeCodeError::ForbiddenTools(_)),
        "{error}"
    );
}

#[tokio::test]
async fn rien_avant_l_init_et_lignes_bornees() {
    let without_init: String = ECHO.lines().skip(1).map(|l| format!("{l}\n")).collect();
    let error = read(&without_init).await.unwrap_err();
    assert!(matches!(error, ClaudeCodeError::Output(_)), "{error}");

    let giant = vec![b'a'; MAX_MESSAGE_BYTES + 1];
    let error = read_events(giant.as_slice()).await.unwrap_err();
    assert!(matches!(error, ClaudeCodeError::Output(_)), "{error}");
}

fn fake(program: &str, args: &[&str]) -> ClaudeCode {
    let mut config = ClaudeCodeConfig::new("/inutile", "Tu es Sherlock.");
    config.program = program.into();
    config.program_args = args.iter().map(|a| (*a).into()).collect();
    config.workdir = std::env::temp_dir().join("sherlockd-tests");
    ClaudeCode::new(config)
}

#[tokio::test]
async fn claude_absent_ou_non_connecte() {
    let error = fake("/chemin/sans/claude", &[])
        .ask("Bonjour")
        .await
        .unwrap_err();
    assert!(matches!(error, ClaudeCodeError::NotInstalled), "{error}");
    // `false auth status` sort avec le code 1, comme `claude auth status` sans connexion.
    let error = fake("false", &[]).ask("Bonjour").await.unwrap_err();
    assert!(matches!(error, ClaudeCodeError::NotLoggedIn), "{error}");
}

#[tokio::test]
async fn historique_borne() {
    // Faux Claude Code : connecté, lit la demande, rejoue la transcription d'echo.
    let script = "[ \"$1\" = auth ] && exit 0; cat > /dev/null; cat \"$0\"";
    let echo = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/transcriptions/echo.jsonl"
    );
    let mut claude = fake("sh", &["-c", script, echo]);
    let first = "a".repeat(15_000);
    for _ in 0..6 {
        let answer = claude.ask(&first).await.unwrap();
        assert!(answer.contains("Bonjour"));
    }
    let size: usize = format!("{:?}", claude.history()).len();
    assert!(size <= MAX_HISTORY_BYTES + 4096, "{size}");
    assert!(claude.history().len() < 6 * 4);
    assert!(claude.prompt("suite").len() <= MAX_HISTORY_BYTES + 1024);
}
