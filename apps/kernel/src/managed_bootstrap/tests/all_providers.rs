//! Row 582 (RED on main): the install plan has a kind that means every provider account of the owner.
use super::*;

fn wire_plan(provider_accounts: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": 1,
        "contextId": "managed_ctx_all_providers",
        "planDigest": format!("sha256:{}", "0".repeat(64)),
        "source": {
            "sourceTargetId": "source-target-test",
            "relayRealmId": "realm-1",
            "machineId": "source-machine-test",
            "kernelId": "source-kernel",
            "keyThumbprint": "a".repeat(64),
        },
        "kernelContext": "empty",
        "developmentSetup": { "kind": "empty" },
        "providerAccounts": provider_accounts,
        "gitCredentials": { "kind": "none" },
    })
}

#[test]
fn the_cloud_wire_plan_can_say_every_provider_account_of_the_owner() {
    let plan: ManagedKernelContextPlan =
        serde_json::from_value(wire_plan(serde_json::json!({ "kind": "all" })))
            .expect("an install plan of kind `all` parses, not as an unknown variant");
    assert_eq!(
        serde_json::to_value(&plan).expect("serialize")["providerAccounts"],
        serde_json::json!({ "kind": "all" }),
        "the kind survives the kernel's own re-serialisation (it is part of the plan digest)"
    );
    assert!(
        !plan.is_empty(),
        "a plan that installs the owner's providers is not an empty plan"
    );
}
