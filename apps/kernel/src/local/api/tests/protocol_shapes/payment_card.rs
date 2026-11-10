use super::*;

// MP-08 / MP-09 / MP-10 / MP-11, local 1002: a saved card's listed details, its any-site use and
// the payment a `vault-card-*` decision approves; the card number and code never appear.
#[test]
fn payment_card_1002_shapes_are_pinned() {
    const { assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 1002) };
    let metadata = crate::config::UserCredentialMetadataConfig {
        use_binding: Some(crate::config::VaultUseBinding {
            target: crate::config::VaultUseTarget::AnySite,
            value_kind: crate::config::VaultValueKind::PaymentCard,
        }),
        payment_card: Some(crate::config::PaymentCardSummary {
            last4: "4242".into(),
            expiry_month: 12,
            expiry_year: 2030,
            name: Some("Work".into()),
            brand: Some("visa".into()),
        }),
        ..Default::default()
    };
    let metadata = serde_json::to_value(&metadata).unwrap();
    assert_eq!(
        metadata["use_binding"],
        serde_json::json!({"target": {"kind": "any_site"}, "value_kind": "payment_card"})
    );
    assert_eq!(
        metadata["payment_card"],
        serde_json::json!({"last4": "4242", "expiry_month": 12, "expiry_year": 2030, "name": "Work", "brand": "visa"})
    );
    let interaction = crate::session::RuntimeInteraction::for_kernel_operation(
        "vault-card-1",
        "operation",
        "Pay with Work?",
        "Fill the card at shop.example",
        vec![
            crate::session::RuntimeInteractionChoice::new("deny", "Cancel", "deny", None),
            crate::session::RuntimeInteractionChoice::new("approve", "Pay", "allow", None)
                .requiring_passkey(),
        ],
    )
    .with_payment(crate::local::VaultPaymentApproval {
        origin: "https://shop.example".into(),
        entry: "Work".into(),
        card_last4: Some("4242".into()),
        card_brand: Some("visa".into()),
        agent_claim: Some(crate::local::VaultPaymentClaim {
            merchant: "Shop".into(),
            amount: "12.50".into(),
            currency: "EUR".into(),
        }),
        requested_by_agent_id: Some("agent".into()),
        session_id: Some("session".into()),
    });
    let mut wire = serde_json::to_value(interaction).unwrap();
    wire["requested_at_ms"] = serde_json::json!(1);
    assert_eq!(
        wire["payment"],
        serde_json::json!({
            "origin": "https://shop.example", "entry": "Work", "card_last4": "4242", "card_brand": "visa",
            "agent_claim": {"merchant": "Shop", "amount": "12.50", "currency": "EUR"},
            "requested_by_agent_id": "agent", "session_id": "session"
        })
    );
    // The claim is the agent's whole statement: a partial claim is refused.
    assert!(
        serde_json::from_value::<crate::local::VaultPaymentApproval>(serde_json::json!({
            "origin": "https://shop.example", "entry": "Work", "agent_claim": {"merchant": "Shop"}
        }))
        .is_err()
    );
    let snapshot = serde_json::json!({"metadata": metadata, "interaction": wire});
    let digest = Sha256::digest(serde_json::to_vec(&snapshot).unwrap());
    assert_eq!(
        format!("{digest:x}"),
        "9e186c42c37290483707528a888cafc80b1fa5863313774cfbba6cce8a724614"
    );
}
