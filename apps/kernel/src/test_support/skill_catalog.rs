use crate::runtime::router::CommandRouter;
use crate::skill::v2::{
    SkillCatalogAction, SkillCatalogMutation, SkillCollisionAction, SkillInstallSource, SkillScope,
    SkillsV2Request, SkillsV2Response,
};

/// Installs one Chariox skill through the same Preview and Mutate requests a
/// client sends, and returns the published catalog revision.
pub(crate) async fn install_skill_through_requests(
    router: &CommandRouter,
    name: &str,
    expected_revision: u64,
) -> u64 {
    let mutation = SkillCatalogMutation {
        scope: SkillScope::Chariox,
        expected_revision,
        actions: vec![SkillCatalogAction::Install {
            source: SkillInstallSource::Content {
                skill_md: format!("---\nname: {name}\ndescription: Probe {name}\n---\nBody.\n"),
            },
            collision: SkillCollisionAction::Reject,
            package_id: None,
        }],
    };
    let state = router.runtime_state();
    let user = crate::session::DEFAULT_LOCAL_USER_ID.to_owned();
    let preview = state
        .execute_skills_catalog_request(
            SkillsV2Request::Preview {
                mutation: mutation.clone(),
            },
            user.clone(),
        )
        .await
        .unwrap();
    let crate::local::LocalDaemonResponse::SkillsV2 {
        result: SkillsV2Response::Preview { review_receipt, .. },
    } = preview
    else {
        panic!("Preview must answer with a review receipt");
    };
    let published = state
        .execute_skills_catalog_request(
            SkillsV2Request::Mutate {
                mutation,
                review_receipt_id: review_receipt.id,
            },
            user,
        )
        .await
        .unwrap();
    let crate::local::LocalDaemonResponse::SkillsV2 {
        result: SkillsV2Response::Published { revision, .. },
    } = published
    else {
        panic!("Mutate must answer with the published revision");
    };
    revision
}
