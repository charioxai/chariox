//! Owner cards of each kind, raised in the access test Room for the grant holder tests.
use super::*;

impl KernelRuntimeState {
    /// One owner card of each kind an external agent meets, in the access test Room; each
    /// answer is written to `answered-<id>` in `root` as the choice the waiting side received.
    pub(super) async fn raise_owner_cards_for_test(&self, root: &std::path::Path) {
        use crate::session::{
            RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionCustomChoice,
            RuntimeInteractionKind, RuntimeInteractionLevel,
        };
        let choices = |approve: &str, label: &str| {
            vec![
                RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
                RuntimeInteractionChoice::new(approve, label, approve, None),
            ]
        };
        let card = |id: &str, title: &str, choices| {
            RuntimeInteraction::for_kernel_operation(id, id, title, "An owner card.", choices)
        };
        let owner_cards = [
            // Asked about a sub-agent two levels below the Room's agent.
            card(
                "capability-request-card",
                "Chariox resource access",
                choices("allow", "Allow"),
            )
            .with_related_agent_id("access-sub-sub-agent".into()),
            card(
                "machine-acquisition-card",
                "Acquire a machine",
                choices("approve", "Acquire"),
            )
            .with_related_agent_id("access-sub-agent".into()),
            RuntimeInteraction::for_kernel_operation(
                "app-install-card",
                "install:card",
                "Install App",
                "Install this App with the declared capabilities?",
                choices("approve", "Install"),
            )
            .with_app_install(AppInstallPresentation {
                request_id: "card".into(),
                operation: AppInstallPromptOperation::Install,
                review: serde_json::json!({}),
            })
            .without_timeout(),
            card(
                "vault-card-payment",
                "Approve card payment",
                vec![
                    RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
                    RuntimeInteractionChoice::new("approve", "Pay", "approve", None)
                        .requiring_passkey(),
                ],
            ),
        ];
        let mut answers = Vec::new();
        for interaction in owner_cards {
            let id = interaction.id().to_owned();
            let answered = self
                .create_kernel_operation_interaction(
                    "access-session",
                    crate::session::DEFAULT_LOCAL_USER_ID,
                    interaction,
                )
                .await
                .unwrap();
            answers.push((id, answered));
        }
        let question = |id: &str, agent: &str, title: &str, choices, custom| {
            RuntimeInteraction::new(
                id,
                agent,
                RuntimeInteractionKind::Choice,
                RuntimeInteractionLevel::Info,
                Some(title.into()),
                "A question for the person.",
                choices,
                custom,
                Some(300),
                None,
            )
        };
        let asked = [
            // A sub-agent asks the person for a secret value.
            (
                "access-session",
                question(
                    "secret-card",
                    "access-sub-agent",
                    "Enter the key",
                    vec![RuntimeInteractionChoice::new("deny", "Deny", "deny", None)],
                    Some(RuntimeInteractionCustomChoice::secret(
                        "value", "Key", None, None, None,
                    )),
                ),
                None,
            ),
            // A sub-agent two levels down asks the person a question.
            (
                "access-session",
                question(
                    "sub-agent-question",
                    "access-sub-sub-agent",
                    "Pick a colour",
                    vec![
                        RuntimeInteractionChoice::new("red", "Red", "red", None),
                        RuntimeInteractionChoice::new("green", "Green", "green", None),
                    ],
                    None,
                ),
                None,
            ),
            // An App's host offer while a user view is open: a card outside any Room.
            (
                "",
                card(
                    "app_host_detached",
                    "Open a link from an App",
                    vec![RuntimeInteractionChoice::new(
                        "decline", "Decline", "deny", None,
                    )],
                ),
                Some(crate::session::DEFAULT_LOCAL_USER_ID),
            ),
        ];
        for (session, interaction, owner) in asked {
            let id = interaction.id().to_owned();
            let (tx, answered) = tokio::sync::oneshot::channel();
            self.owned
                .register_runtime_interaction(session, interaction, tx, owner, None, None)
                .unwrap();
            answers.push((id, answered));
        }
        for (id, answered) in answers {
            let file = root.join(format!("answered-{id}"));
            tokio::spawn(async move {
                if let Ok(answer) = answered.await {
                    std::fs::write(file, serde_json::json!(answer.choice_id).to_string()).unwrap();
                }
            });
        }
    }
}
